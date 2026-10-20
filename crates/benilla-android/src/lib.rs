//! The Android launcher: NativeActivity calls `android_main` on its own thread, which hands the
//! activity and the build stamp to `benilla_app::android`, as the `benilla` shim's `main` does.

#![cfg(target_os = "android")]

use benilla_app::{android::AndroidApp, BuildId};

#[no_mangle]
fn android_main(app: AndroidApp) {
    benilla_app::android::main(
        BuildId {
            version: env!("CARGO_PKG_VERSION"),
            describe: env!("BENILLA_GIT_DESCRIBE"),
            sha: env!("BENILLA_GIT_SHA"),
            short: env!("BENILLA_GIT_SHORT"),
            date: env!("BENILLA_GIT_DATE"),
            profile: env!("BENILLA_PROFILE"),
            project_dir: env!("BENILLA_PROJECT_DIR"),
            ..Default::default()
        },
        app,
    );
}
