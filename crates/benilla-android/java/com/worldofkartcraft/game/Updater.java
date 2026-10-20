package com.worldofkartcraft.game;

import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInstaller;

import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * Hands a downloaded APK of this app to Android's installer, which asks the player to confirm
 * the update. Called from Rust (benilla-app/src/android.rs, update) through JNI; the result comes
 * back to {@link InstallReceiver}.
 */
public final class Updater {
    private Updater() {}

    public static void install(Context context, String apk) throws IOException {
        PackageInstaller installer = context.getPackageManager().getPackageInstaller();
        PackageInstaller.SessionParams params =
                new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
        params.setAppPackageName(context.getPackageName());
        int id = installer.createSession(params);
        try (PackageInstaller.Session session = installer.openSession(id)) {
            File file = new File(apk);
            try (InputStream in = new FileInputStream(file);
                    OutputStream out = session.openWrite("base.apk", 0, file.length())) {
                byte[] buf = new byte[1 << 16];
                int n;
                while ((n = in.read(buf)) > 0) {
                    out.write(buf, 0, n);
                }
                session.fsync(out);
            }
            Intent done = new Intent(context, InstallReceiver.class);
            // Mutable: the installer fills in the status and the confirmation to show.
            PendingIntent result = PendingIntent.getBroadcast(context, id, done,
                    PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_MUTABLE);
            session.commit(result.getIntentSender());
        }
    }
}
