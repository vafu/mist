package io.github.vafu.mist.android;

import android.os.Bundle;

oneway interface IMistResultCallback {
    void onSuccess(in Bundle result);
    void onFailure(String message);
}
