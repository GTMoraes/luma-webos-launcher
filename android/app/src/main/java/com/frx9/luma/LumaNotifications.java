package com.frx9.luma;

import android.app.ActivityOptions;
import android.app.Notification;
import android.app.PendingIntent;
import android.content.ComponentName;
import android.content.Context;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.service.notification.NotificationListenerService;
import android.service.notification.StatusBarNotification;
import android.util.Log;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/**
 * Lets Luma's quick settings list and open notifications, as the Google TV
 * panel does. Android runs it only after Notification access is granted
 * (Settings › Apps › Special app access, or the panel's own row).
 */
public class LumaNotifications extends NotificationListenerService {
    private static final String TAG = "Luma";
    private static volatile LumaNotifications connected;

    @Override
    public void onListenerConnected() {
        connected = this;
    }

    @Override
    public void onListenerDisconnected() {
        connected = null;
    }

    static boolean allowed(Context context) {
        String enabled = Settings.Secure.getString(
                context.getContentResolver(), "enabled_notification_listeners");
        ComponentName self = new ComponentName(context, LumaNotifications.class);
        return enabled != null && enabled.contains(self.flattenToString());
    }

    /** Recent notifications with something to show, newest first. */
    static List<StatusBarNotification> recent() {
        LumaNotifications service = connected;
        if (service == null) {
            return Collections.emptyList();
        }
        List<StatusBarNotification> list = new ArrayList<>();
        try {
            for (StatusBarNotification item : service.getActiveNotifications()) {
                Notification notification = item.getNotification();
                if (item.isOngoing()
                        || (notification.flags & Notification.FLAG_GROUP_SUMMARY) != 0
                        || notification.extras.getCharSequence(Notification.EXTRA_TITLE) == null) {
                    continue;
                }
                list.add(item);
            }
        } catch (Exception e) {
            Log.w(TAG, "Notifications", e);
        }
        list.sort((a, b) -> Long.compare(b.getPostTime(), a.getPostTime()));
        return list;
    }

    static boolean open(String key) {
        LumaNotifications service = connected;
        if (service == null) {
            return false;
        }
        try {
            for (StatusBarNotification item : service.getActiveNotifications(new String[] {key})) {
                PendingIntent content = item.getNotification().contentIntent;
                if ((item.getNotification().flags & Notification.FLAG_AUTO_CANCEL) != 0) {
                    service.cancelNotification(key);
                }
                if (content != null) {
                    // Luma is on screen: let the notification's app open in front.
                    Bundle options = null;
                    if (Build.VERSION.SDK_INT >= 34) {
                        options = ActivityOptions.makeBasic()
                                .setPendingIntentBackgroundActivityStartMode(
                                        ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED)
                                .toBundle();
                    }
                    content.send(service, 0, null, null, null, null, options);
                    return true;
                }
            }
        } catch (Exception e) {
            Log.w(TAG, "Could not open notification " + key, e);
        }
        return false;
    }

    static boolean clear() {
        LumaNotifications service = connected;
        if (service == null) {
            return false;
        }
        try {
            service.cancelAllNotifications();
            return true;
        } catch (Exception e) {
            Log.w(TAG, "Could not clear notifications", e);
            return false;
        }
    }
}
