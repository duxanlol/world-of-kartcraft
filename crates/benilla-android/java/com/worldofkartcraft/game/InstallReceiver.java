package com.worldofkartcraft.game;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInstaller;
import android.util.Log;

/**
 * The installer's answer to {@link Updater}: an app that did not install itself must have the
 * player confirm, so the confirmation the installer sends is shown; anything else is logged.
 */
public final class InstallReceiver extends BroadcastReceiver {
    // The one-argument getParcelableExtra: its typed successor is Android 13's, the APK runs on 11.
    @Override
    @SuppressWarnings("deprecation")
    public void onReceive(Context context, Intent intent) {
        int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
        if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
            Intent confirm = intent.getParcelableExtra(Intent.EXTRA_INTENT);
            if (confirm != null) {
                confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                context.startActivity(confirm);
            }
            return;
        }
        Log.i("benilla", "update: install status " + status + ": "
                + intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE));
    }
}
