#[cfg(target_os = "android")]
#[allow(non_snake_case)]
#[no_mangle]
pub extern "system" fn Java_de_tbuck_satchel_MainActivity_initializePlatformTls(
    mut env: jni::JNIEnv<'_>,
    activity: jni::objects::JObject<'_>,
) -> jni::sys::jboolean {
    match rustls_platform_verifier::android::init_with_env(&mut env, activity) {
        Ok(()) => jni::sys::JNI_TRUE,
        Err(error) => {
            tracing::error!(%error, "failed to initialize Android platform TLS verification");
            jni::sys::JNI_FALSE
        }
    }
}
