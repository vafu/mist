package io.github.vafu.mist.android;

import android.os.Bundle;

oneway interface IMistCallback {
    void onStateChanged(in Bundle state);
    void onClipboardReceived(in Bundle content);
    void onNotificationAction(in Bundle activation);
}
