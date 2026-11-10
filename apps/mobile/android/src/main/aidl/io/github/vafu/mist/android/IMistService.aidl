package io.github.vafu.mist.android;

import android.os.Bundle;
import io.github.vafu.mist.android.IMistCallback;
import io.github.vafu.mist.android.IMistResultCallback;

interface IMistService {
    Bundle snapshot();
    void registerCallback(IMistCallback callback);
    void unregisterCallback(IMistCallback callback);
    void initiatePairing(IMistResultCallback callback);
    void cancelPairing();
    void acceptPairing(String offer, IMistResultCallback callback);
    void selectDevice(String endpointId, boolean selected, IMistResultCallback callback);
    void connectDevice(String endpointId, IMistResultCallback callback);
    void disconnectDevice(String endpointId, IMistResultCallback callback);
    void forgetDevice(String endpointId, IMistResultCallback callback);
    void pushClipboard(in Bundle content, in String[] targets, IMistResultCallback callback);
    void shareFile(String path, in String[] targets, IMistResultCallback callback);
    void activateNotification(String sourceEndpointId, String notificationId, String actionId,
        IMistResultCallback callback);
    oneway void pushNotification(in Bundle notification);
}
