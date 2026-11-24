//! Android TV build of Luma: SDLActivity loads this library (libluma.so) and
//! enters through `SDL_main` in android.rs. On every other target the library
//! is empty and the `luma` executable is built from main.rs.
#![cfg(target_os = "android")]
// The shared modules keep their webOS and desktop paths, unused on Android.
#![allow(dead_code, unused_imports)]

include!("main.rs");
