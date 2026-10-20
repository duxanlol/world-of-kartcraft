package com.worldofkartcraft.game;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;
import android.os.Process;

/**
 * Starts the app again: the game loads its kart data at start, so after converting the disc
 * (benilla-app/src/android.rs, convert) it has to. This activity runs in a process of its own
 * (":restart" in the manifest): it ends the game's process, launches the game afresh, and ends
 * its own.
 */
public final class Phoenix extends Activity {
    private static final String PID = "pid";

    /** Called from Rust through JNI: hands over to the restart process. */
    public static void rebirth(Context context) {
        Intent intent = new Intent(context, Phoenix.class);
        intent.putExtra(PID, Process.myPid());
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        context.startActivity(intent);
    }

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        int pid = getIntent().getIntExtra(PID, -1);
        if (pid > 0) {
            Process.killProcess(pid);
        }
        Intent launch = getPackageManager().getLaunchIntentForPackage(getPackageName());
        if (launch != null) {
            launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK);
            startActivity(launch);
        }
        finish();
        Runtime.getRuntime().exit(0);
    }
}
