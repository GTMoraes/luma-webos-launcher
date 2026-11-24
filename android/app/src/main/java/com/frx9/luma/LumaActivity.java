package com.frx9.luma;

import android.content.ComponentName;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.ResolveInfo;
import android.content.res.AssetManager;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.drawable.Drawable;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.Uri;
import android.net.wifi.WifiInfo;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.Bundle;
import android.os.SystemClock;
import android.provider.Settings;
import android.service.notification.StatusBarNotification;
import android.system.Os;
import android.util.Log;
import android.view.KeyEvent;

import org.json.JSONArray;
import org.json.JSONObject;
import org.libsdl.app.SDLActivity;

import java.io.BufferedReader;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Luma on Android TV. SDL runs the Rust launcher (libluma.so); this activity
 * prepares its folders and answers the app list, artwork and launch calls
 * that android.rs makes over JNI, from Luma's worker threads.
 */
public class LumaActivity extends SDLActivity {
    private static final String TAG = "Luma";

    @Override
    protected String[] getLibraries() {
        return new String[] {"SDL2", "luma"};
    }

    @Override
    protected void onCreate(Bundle state) {
        // Paths for android.rs, set before SDL loads the library and starts it.
        try {
            File shared = getExternalFilesDir(null);
            if (shared == null) {
                shared = getFilesDir();
            }
            shared.mkdirs();
            Os.setenv("LUMA_DATA", shared.getAbsolutePath(), true);
            Os.setenv("LUMA_PRIVATE", getFilesDir().getAbsolutePath(), true);
            Os.setenv("LUMA_ASSETS", unpackAssets().getAbsolutePath(), true);
        } catch (Exception e) {
            Log.e(TAG, "Luma could not prepare its files", e);
        }
        super.onCreate(state);
    }

    /**
     * Luma reads its fonts, wallpapers and vector artwork from files. They are
     * copied out of the APK once per installed version.
     */
    private File unpackAssets() throws Exception {
        File target = new File(getFilesDir(), "luma-assets");
        PackageInfo info = getPackageManager().getPackageInfo(getPackageName(), 0);
        String stamp = info.versionName + "-" + info.lastUpdateTime;
        File marker = new File(target, ".unpacked");
        if (marker.isFile()
                && stamp.equals(new String(Files.readAllBytes(marker.toPath()), StandardCharsets.UTF_8))) {
            return target;
        }
        long started = SystemClock.uptimeMillis();
        deleteTree(target);
        AssetManager assets = getAssets();
        byte[] buffer = new byte[1 << 16];
        int count = 0;
        try (BufferedReader list = new BufferedReader(
                new InputStreamReader(assets.open("luma/files.txt"), StandardCharsets.UTF_8))) {
            String line;
            while ((line = list.readLine()) != null) {
                if (line.isEmpty() || line.startsWith("/") || line.contains("..")) {
                    continue;
                }
                File out = new File(target, line);
                File parent = out.getParentFile();
                if (parent != null) {
                    parent.mkdirs();
                }
                try (InputStream in = assets.open("luma/" + line);
                        OutputStream file = new FileOutputStream(out)) {
                    int read;
                    while ((read = in.read(buffer)) > 0) {
                        file.write(buffer, 0, read);
                    }
                }
                count++;
            }
        }
        try (OutputStream file = new FileOutputStream(marker)) {
            file.write(stamp.getBytes(StandardCharsets.UTF_8));
        }
        Log.i(TAG, "Unpacked " + count + " asset files in "
                + (SystemClock.uptimeMillis() - started) + " ms");
        return target;
    }

    private static void deleteTree(File file) {
        File[] children = file.listFiles();
        if (children != null) {
            for (File child : children) {
                deleteTree(child);
            }
        }
        file.delete();
    }

    /**
     * The remote's OK arrives as DPAD_CENTER, which SDL maps to SELECT; Luma
     * expects Return. The D-pad, Back and Menu go straight to SDL's keyboard,
     * so a remote that also reports itself as a gamepad still drives Luma.
     */
    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        int code;
        switch (event.getKeyCode()) {
            case KeyEvent.KEYCODE_DPAD_CENTER:
            case KeyEvent.KEYCODE_ENTER:
            case KeyEvent.KEYCODE_NUMPAD_ENTER:
            case KeyEvent.KEYCODE_BUTTON_A:
                code = KeyEvent.KEYCODE_ENTER;
                break;
            case KeyEvent.KEYCODE_BACK:
            case KeyEvent.KEYCODE_BUTTON_B:
                code = KeyEvent.KEYCODE_BACK;
                break;
            case KeyEvent.KEYCODE_DPAD_UP:
            case KeyEvent.KEYCODE_DPAD_DOWN:
            case KeyEvent.KEYCODE_DPAD_LEFT:
            case KeyEvent.KEYCODE_DPAD_RIGHT:
            case KeyEvent.KEYCODE_MENU:
                code = event.getKeyCode();
                break;
            default:
                return super.dispatchKeyEvent(event);
        }
        if (SDLActivity.mBrokenLibraries) {
            return super.dispatchKeyEvent(event);
        }
        if (event.getAction() == KeyEvent.ACTION_DOWN) {
            SDLActivity.onNativeKeyDown(code);
        } else if (event.getAction() == KeyEvent.ACTION_UP) {
            SDLActivity.onNativeKeyUp(code);
        }
        return true;
    }

    // ------------------------------------------------------------ JNI calls

    /** Apps with a TV (Leanback) or ordinary launcher entry, as JSON. */
    public String lumaApps() {
        PackageManager pm = getPackageManager();
        Map<String, ApplicationInfo> found = new LinkedHashMap<>();
        for (String category : new String[] {
                Intent.CATEGORY_LEANBACK_LAUNCHER, Intent.CATEGORY_LAUNCHER}) {
            Intent query = new Intent(Intent.ACTION_MAIN).addCategory(category);
            for (ResolveInfo entry : pm.queryIntentActivities(query, 0)) {
                ApplicationInfo app = entry.activityInfo.applicationInfo;
                if (!app.packageName.equals(getPackageName()) && !found.containsKey(app.packageName)) {
                    found.put(app.packageName, app);
                }
            }
        }
        JSONArray list = new JSONArray();
        for (ApplicationInfo app : found.values()) {
            try {
                JSONObject item = new JSONObject();
                item.put("id", app.packageName);
                item.put("title", String.valueOf(pm.getApplicationLabel(app)));
                item.put("category", category(app));
                String version = "";
                try {
                    PackageInfo info = pm.getPackageInfo(app.packageName, 0);
                    version = info.versionName + "-" + info.lastUpdateTime;
                } catch (PackageManager.NameNotFoundException ignored) {
                    // Removed while listing: it still gets a card until the next refresh.
                }
                item.put("version", version);
                list.put(item);
            } catch (Exception e) {
                Log.w(TAG, "App " + app.packageName + " left out", e);
            }
        }
        return list.toString();
    }

    /** Luma's Apps categories, for apps without curated artwork. */
    private static String category(ApplicationInfo app) {
        if ((app.flags & ApplicationInfo.FLAG_IS_GAME) != 0) {
            return "Gaming";
        }
        if (Build.VERSION.SDK_INT >= 26) {
            switch (app.category) {
                case ApplicationInfo.CATEGORY_GAME:
                    return "Gaming";
                case ApplicationInfo.CATEGORY_AUDIO:
                    return "Music";
                case ApplicationInfo.CATEGORY_VIDEO:
                    return "Streaming";
                default:
                    break;
            }
        }
        return "Utilities";
    }

    /** The app's TV banner, or its icon without one, as PNG; null if missing. */
    public byte[] lumaArt(String id) {
        try {
            PackageManager pm = getPackageManager();
            Drawable art = null;
            Intent tv = pm.getLeanbackLaunchIntentForPackage(id);
            if (tv != null && tv.getComponent() != null) {
                art = pm.getActivityBanner(tv.getComponent());
            }
            if (art == null) {
                art = pm.getApplicationBanner(id);
            }
            int width = 192;
            int height = 192;
            if (art != null) {
                width = art.getIntrinsicWidth();
                height = art.getIntrinsicHeight();
                if (width <= 0 || height <= 0) {
                    width = 320;
                    height = 180;
                }
                // Banners are 16:9; Luma shows them at about 180 px wide.
                if (width > 480) {
                    height = Math.max(1, height * 480 / width);
                    width = 480;
                }
            } else {
                art = pm.getApplicationIcon(id);
            }
            Bitmap bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888);
            Canvas canvas = new Canvas(bitmap);
            art.setBounds(0, 0, width, height);
            art.draw(canvas);
            ByteArrayOutputStream png = new ByteArrayOutputStream();
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, png);
            bitmap.recycle();
            return png.toByteArray();
        } catch (Exception e) {
            Log.w(TAG, "No artwork for " + id, e);
            return null;
        }
    }

    /**
     * Opens an app by package or an activity given as package/.Activity. Also:
     * "action:" a settings page, "uninstall:" a package (the system asks first),
     * "notification:" a notification by key, and "notifications:clear".
     */
    public boolean lumaOpen(String target) {
        PackageManager pm = getPackageManager();
        Intent intent;
        if (target.startsWith("notification:")) {
            return LumaNotifications.open(target.substring("notification:".length()));
        } else if (target.equals("notifications:clear")) {
            return LumaNotifications.clear();
        } else if (target.startsWith("action:")) {
            intent = new Intent(target.substring("action:".length()));
            if (pm.resolveActivity(intent, 0) == null) {
                intent = new Intent(Settings.ACTION_SETTINGS);
            }
        } else if (target.startsWith("uninstall:")) {
            intent = new Intent(Intent.ACTION_DELETE,
                    Uri.parse("package:" + target.substring("uninstall:".length())));
        } else if (target.indexOf('/') > 0) {
            ComponentName component = ComponentName.unflattenFromString(target);
            if (component == null) {
                return false;
            }
            intent = new Intent(Intent.ACTION_MAIN).setComponent(component);
            if (pm.resolveActivity(intent, 0) == null) {
                return false;
            }
        } else {
            intent = pm.getLeanbackLaunchIntentForPackage(target);
            if (intent == null) {
                intent = pm.getLaunchIntentForPackage(target);
            }
            if (intent == null) {
                return false;
            }
        }
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        final Intent launch = intent;
        runOnUiThread(() -> {
            try {
                startActivity(launch);
            } catch (Exception e) {
                Log.w(TAG, "Could not open " + target, e);
            }
        });
        return true;
    }

    /** The connection and recent notifications for the quick settings panel, as JSON. */
    public String lumaQuick() {
        JSONObject reply = new JSONObject();
        try {
            String network = "none";
            int signal = 0;
            ConnectivityManager connectivity = getSystemService(ConnectivityManager.class);
            Network active = connectivity == null ? null : connectivity.getActiveNetwork();
            NetworkCapabilities caps =
                    active == null ? null : connectivity.getNetworkCapabilities(active);
            if (caps != null && caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) {
                network = "ethernet";
            } else if (caps != null && caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) {
                network = "wifi";
                WifiManager wifi = getSystemService(WifiManager.class);
                WifiInfo info = wifi == null ? null : wifi.getConnectionInfo();
                if (info != null) {
                    signal = WifiManager.calculateSignalLevel(info.getRssi(), 5);
                }
            }
            reply.put("network", network);
            reply.put("signal", signal);
            boolean allowed = LumaNotifications.allowed(this);
            reply.put("allowed", allowed);
            JSONArray notices = new JSONArray();
            PackageManager pm = getPackageManager();
            for (StatusBarNotification item : LumaNotifications.recent()) {
                JSONObject notice = new JSONObject();
                notice.put("key", item.getKey());
                String app = item.getPackageName();
                try {
                    app = String.valueOf(pm.getApplicationLabel(pm.getApplicationInfo(app, 0)));
                } catch (PackageManager.NameNotFoundException ignored) {
                    // The package name stands in for its label.
                }
                notice.put("app", app);
                CharSequence title = item.getNotification().extras.getCharSequence("android.title");
                CharSequence text = item.getNotification().extras.getCharSequence("android.text");
                notice.put("title", title == null ? "" : title.toString());
                notice.put("text", text == null ? "" : text.toString());
                notices.put(notice);
            }
            reply.put("notices", notices);
        } catch (Exception e) {
            Log.w(TAG, "Quick settings", e);
        }
        return reply.toString();
    }
}
