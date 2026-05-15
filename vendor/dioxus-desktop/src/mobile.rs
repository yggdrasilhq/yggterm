/// Expose the `Java_dev_dioxus_main_Rust_*` functions to the JNI layer.
/// We hardcode these to have a single trampoline for host Java code to call into.
///
/// This saves us from having to plumb the top-level package name all the way down into
/// this file. This is better for modularity (ie just call dioxus' main to run the app) as
/// well as cache thrashing since this crate doesn't rely on external env vars.
///
/// The CLI is expecting to find `dev.dioxus.main` in the final library. If you find a need to
/// change this, you'll need to change the CLI as well.
#[cfg(target_os = "android")]
#[no_mangle]
#[inline(never)]
pub extern "C" fn start_app() {
    use tao::platform::android::prelude::ndk::looper::ThreadLooper;
    use tao::platform::android::prelude::{
        android_fn, create as tao_create, generate_package_name,
        onActivityCreate as tao_on_activity_create, onActivityDestroy as tao_on_activity_destroy,
        onActivityLowMemory as tao_on_activity_low_memory,
        onActivitySaveInstanceState as tao_on_activity_save_instance_state,
        onNewIntent as tao_on_new_intent, onWindowFocusChanged as tao_on_window_focus_changed,
        pause as tao_pause, resume as tao_resume, start as tao_start, stop as tao_stop, GlobalRef,
        JClass, JNIEnv, JObject, PACKAGE,
    };
    wry::android_binding!(dev_dioxus, main, wry);

    fn store_package_name() {
        PACKAGE.get_or_init(move || generate_package_name!(dev_dioxus, main));
    }

    unsafe fn create<'local>(env: JNIEnv<'local>, class: JClass<'local>, main: fn()) {
        tao_create(env, class, JObject::null(), main);
    }

    #[allow(non_snake_case)]
    unsafe fn onActivityCreate<'local>(
        env: JNIEnv<'local>,
        class: JClass<'local>,
        activity: JObject<'local>,
        setup: unsafe fn(&str, JNIEnv, &ThreadLooper, GlobalRef),
    ) {
        static NDK_CONTEXT_READY: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);

        if !NDK_CONTEXT_READY.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let vm = env
                .get_java_vm()
                .expect("Android Java VM should be available");
            let activity_ref = env
                .new_global_ref(&activity)
                .expect("Android activity global ref should be available");
            unsafe {
                ndk_context::initialize_android_context(
                    vm.get_java_vm_pointer() as *mut _,
                    activity_ref.as_obj().as_raw() as *mut _,
                );
            }
            std::mem::forget(activity_ref);
        }

        tao_on_activity_create(env, class, activity, setup);
    }

    unsafe fn start<'local>(env: JNIEnv<'local>, class: JClass<'local>) {
        tao_start(env, class, JObject::null());
    }

    unsafe fn stop<'local>(env: JNIEnv<'local>, class: JClass<'local>) {
        tao_stop(env, class, JObject::null());
    }

    unsafe fn resume<'local>(env: JNIEnv<'local>, class: JClass<'local>) {
        tao_resume(env, class, JObject::null());
    }

    unsafe fn pause<'local>(env: JNIEnv<'local>, class: JClass<'local>) {
        tao_pause(env, class, JObject::null());
    }

    #[allow(non_snake_case)]
    unsafe fn onActivitySaveInstanceState<'local>(env: JNIEnv<'local>, class: JClass<'local>) {
        tao_on_activity_save_instance_state(env, class, JObject::null());
    }

    #[allow(non_snake_case)]
    unsafe fn onActivityLowMemory<'local>(env: JNIEnv<'local>, class: JClass<'local>) {
        tao_on_activity_low_memory(env, class, JObject::null());
    }

    #[allow(non_snake_case)]
    unsafe fn onActivityDestroy<'local>(
        env: JNIEnv<'local>,
        class: JClass<'local>,
        activity: JObject<'local>,
    ) {
        tao_on_activity_destroy(env, class, activity);
    }

    #[allow(non_snake_case)]
    unsafe fn onWindowFocusChanged<'local>(
        env: JNIEnv<'local>,
        class: JClass<'local>,
        activity: JObject<'local>,
        has_focus: i32,
    ) {
        tao_on_window_focus_changed(env, class, activity, has_focus);
    }

    #[allow(non_snake_case)]
    unsafe fn onNewIntent<'local>(
        env: JNIEnv<'local>,
        class: JClass<'local>,
        intent: JObject<'local>,
    ) {
        tao_on_new_intent(env, class, intent);
    }

    #[no_mangle]
    unsafe extern "C" fn Java_dev_dioxus_main_Rust_create<'local>(
        env: JNIEnv<'local>,
        class: JClass<'local>,
    ) {
        store_package_name();
        create(env, class, root);
    }
    android_fn!(
        dev_dioxus,
        main,
        Rust,
        onActivityCreate,
        [JObject<'local>],
        __VOID__,
        [wry::android_setup],
        store_package_name,
    );
    android_fn!(dev_dioxus, main, Rust, start, []);
    android_fn!(dev_dioxus, main, Rust, stop, []);
    android_fn!(dev_dioxus, main, Rust, resume, []);
    android_fn!(dev_dioxus, main, Rust, pause, []);
    android_fn!(dev_dioxus, main, Rust, onActivitySaveInstanceState, []);
    android_fn!(dev_dioxus, main, Rust, onActivityLowMemory, []);
    android_fn!(dev_dioxus, main, Rust, onActivityDestroy, [JObject<'local>]);
    android_fn!(
        dev_dioxus,
        main,
        Rust,
        onWindowFocusChanged,
        [JObject<'local>, i32]
    );
    android_fn!(dev_dioxus, main, Rust, onNewIntent, [JObject<'local>]);

    #[cfg(target_os = "android")]
    fn root() {
        fn stop_unwind<F: FnOnce() -> T, T>(f: F) -> T {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
                Ok(t) => t,
                Err(err) => {
                    eprintln!("attempt to unwind out of `rust` with err: {:?}", err);
                    std::process::abort()
                }
            }
        }

        stop_unwind(|| unsafe {
            let mut main_fn_ptr = libc::dlsym(libc::RTLD_DEFAULT, b"main\0".as_ptr() as _);

            if main_fn_ptr.is_null() {
                main_fn_ptr = libc::dlsym(libc::RTLD_DEFAULT, b"_main\0".as_ptr() as _);
            }

            if main_fn_ptr.is_null() {
                panic!("Failed to find main symbol");
            }

            // Set the env vars that rust code might expect, passed off to us by the android app
            // Doing this before main emulates the behavior of a regular executable
            if cfg!(target_os = "android") && cfg!(debug_assertions) {
                // Load the env file from the session cache if we're in debug mode and on android
                //
                // This is a slightly hacky way of being able to use std::env::var code in android apps without
                // going through their custom java-based system.
                let env_file = dioxus_cli_config::android_session_cache_dir().join(".env");
                if let Ok(env_file) = std::fs::read_to_string(&env_file) {
                    for line in env_file.lines() {
                        if let Some((key, value)) = line.trim().split_once('=') {
                            std::env::set_var(key, value);
                        }
                    }
                }
            }

            let main_fn: extern "C" fn() = std::mem::transmute(main_fn_ptr);
            main_fn();
        });
    }
}
