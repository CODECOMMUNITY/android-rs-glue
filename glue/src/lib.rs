#![feature(box_syntax)]
#![feature(plugin)]

#![unstable]

#[plugin]
#[no_link]
extern crate compile_msg;

extern crate libc;

use std::ffi::{CString};
use std::sync::mpsc::{Sender};
use std::sync::Mutex;
use std::thread::Thread;

#[doc(hidden)]
pub mod ffi;

/// This static variable  will store the android_app* on creation, and set it back to 0 at
///  destruction.
/// Apart from this, the static is never written, so there is no risk of race condition.
static mut ANDROID_APP: *mut ffi::android_app = 0 as *mut ffi::android_app;

/// This is the structure that serves as user data in the android_app*
#[doc(hidden)]
struct Context {
    senders: Mutex<Vec<Sender<Event>>>,
}

/// An event triggered by the Android environment.
pub enum Event {
    EventUp,
    EventDown,
    EventMove(i32, i32),
}

impl Copy for Event {}

#[cfg(not(target_os = "android"))]
compile_note!("You are not compiling for Android");

#[macro_export]
macro_rules! android_start(
    ($main: ident) => (
        pub mod __android_start {
            extern crate android_glue;

            // this function is here because we are sure that it will be included by the linker
            // so we call app_dummy in it, in order to be sure that the native glue will be included
            #[start]
            pub fn start(_: int, _: *const *const u8) -> int {
                unsafe { android_glue::ffi::app_dummy() };
                1
            }

            #[no_mangle]
            #[inline(never)]
            #[allow(non_snake_case)]
            pub extern "C" fn android_main(app: *mut ()) {
                android_glue::android_main2(app, move|| ::$main());
            }
        }
    )
);

/// This is the function that must be called by `android_main`
#[doc(hidden)]
pub fn android_main2<F>(app: *mut (), main_function: F)
    where F: FnOnce(), F: Send
{
    use std::{mem, ptr};

    write_log("Entering android_main");

    unsafe { ANDROID_APP = std::mem::transmute(app) };
    let app: &mut ffi::android_app = unsafe { std::mem::transmute(app) };

    // creating the context that will be passed to the callback
    let context = Context { senders: Mutex::new(Vec::new()) };
    app.onAppCmd = commands_callback;
    app.onInputEvent = inputs_callback;
    app.userData = unsafe { std::mem::transmute(&context) };

    // executing the main function in parallel
    let g = Thread::spawn(move|| {
        std::io::stdio::set_stdout(box std::io::LineBufferedWriter::new(ToLogWriter));
        std::io::stdio::set_stderr(box std::io::LineBufferedWriter::new(ToLogWriter));
        main_function()
    });

    // polling for events forever
    // note that this must be done in the same thread as android_main because ALooper are
    //  thread-local
    unsafe {
        loop {
            let mut events = mem::uninitialized();
            let mut source = mem::uninitialized();

            // passing -1 means that we are blocking
            let ident = ffi::ALooper_pollAll(-1, ptr::null_mut(), &mut events,
                &mut source);

            // processing the event
            if !source.is_null() {
                let source: *mut ffi::android_poll_source = mem::transmute(source);
                ((*source).process)(ANDROID_APP, source);
            }
        }
    }

    // terminating the application
    unsafe { ANDROID_APP = 0 as *mut ffi::android_app };
}

/// Writer that will redirect what is written to it to the logs.
struct ToLogWriter;

impl Writer for ToLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::IoResult<()> {
        let message = CString::from_slice(buf).as_slice_with_nul().as_ptr();
        let tag = b"RustAndroidGlueStdouterr";
        let tag = CString::from_slice(tag).as_slice_with_nul().as_ptr();
        unsafe { ffi::__android_log_write(3, tag, message) };
        Ok(())
    }
}

/// The callback for inputs.
pub extern fn inputs_callback(_: *mut ffi::android_app, event: *const ffi::AInputEvent)
    -> libc::int32_t
{
    fn send_event(event: Event) {
        let senders = get_context().senders.lock().ok().unwrap();
        for sender in senders.iter() {
            sender.send(event);
        }
    }
    fn get_xy(event: *const ffi::AInputEvent) -> (i32, i32) {
        let x = unsafe { ffi::AMotionEvent_getX(event, 0) };
        let y = unsafe { ffi::AMotionEvent_getY(event, 0) };
        (x as i32, y as i32)
    }
    let action = unsafe { ffi::AMotionEvent_getAction(event) };
    let action_code = action & ffi::AMOTION_EVENT_ACTION_MASK;
    match action_code {
        ffi::AMOTION_EVENT_ACTION_UP
            | ffi::AMOTION_EVENT_ACTION_OUTSIDE
            | ffi::AMOTION_EVENT_ACTION_CANCEL
            | ffi::AMOTION_EVENT_ACTION_POINTER_UP =>
        {
            send_event(Event::EventUp);
        },
        ffi::AMOTION_EVENT_ACTION_DOWN
            | ffi::AMOTION_EVENT_ACTION_POINTER_DOWN =>
        {
            let (x, y) = get_xy(event);
            send_event(Event::EventMove(x, y));
            send_event(Event::EventDown);
        },
        _ => {
            let (x, y) = get_xy(event);
            send_event(Event::EventMove(x, y));
        },
    }
    0
}

/// The callback for commands.
#[doc(hidden)]
pub extern fn commands_callback(_: *mut ffi::android_app, command: libc::int32_t) {
    let context = get_context();

    match command {
        ffi::APP_CMD_INIT_WINDOW => {

        },

        ffi::APP_CMD_SAVE_STATE => {

        },

        ffi::APP_CMD_TERM_WINDOW => {

        },

        ffi::APP_CMD_GAINED_FOCUS => {

        },

        ffi::APP_CMD_LOST_FOCUS => {

        },

        _ => ()
    }
}

/// Returns the current Context.
fn get_context() -> &'static Context {
    let context = unsafe { (*ANDROID_APP).userData };
    unsafe { std::mem::transmute(context) }
}

/// Adds a sender where events will be sent to.
pub fn add_sender(sender: Sender<Event>) {
    get_context().senders.lock().ok().unwrap().push(sender);
}

/// Returns a handle to the native window.
pub unsafe fn get_native_window() -> ffi::NativeWindowType {
    if ANDROID_APP.is_null() {
        panic!("The application was not initialized from android_main");
    }

    loop {
        let value = (*ANDROID_APP).window;
        if !value.is_null() {
            return value;
        }

        // spin-locking
        std::io::timer::sleep(std::time::Duration::milliseconds(10));
    }
}

/// 
pub fn write_log(message: &str) {
    let message = message.as_bytes();
    let message = CString::from_slice(message).as_slice_with_nul().as_ptr();
    let tag = b"RustAndroidGlueStdouterr";
    let tag = CString::from_slice(tag).as_slice_with_nul().as_ptr();
    unsafe { ffi::__android_log_write(3, tag, message) };
}

pub enum AssetError {
    AssetMissing,
    EmptyBuffer,
}

pub fn load_asset(filename: &str) -> Result<Vec<u8>, AssetError> {
    struct AssetCloser {
        asset: *const ffi::Asset,
    }

    impl Drop for AssetCloser {
        fn drop(&mut self) {
            unsafe {
                ffi::AAsset_close(self.asset)
            };
        }
    }

    unsafe fn get_asset_manager() -> *const ffi::AAssetManager {
        let app = &*ANDROID_APP;
        let activity = &*app.activity;
        activity.assetManager
    }

    let filename_c_str = CString::from_slice(filename.as_bytes())
        .as_slice_with_nul().as_ptr();
    let asset = unsafe {
        ffi::AAssetManager_open(
            get_asset_manager(), filename_c_str, ffi::MODE_STREAMING)
    };
    if asset.is_null() {
        return Err(AssetError::AssetMissing);
    }
    let _asset_closer = AssetCloser{asset: asset};
    let len = unsafe {
        ffi::AAsset_getLength(asset)
    };
    let buff = unsafe {
        ffi::AAsset_getBuffer(asset)
    };
    if buff.is_null() {
        return Err(AssetError::EmptyBuffer);
    }
    let vec = unsafe {
        Vec::from_raw_buf(buff as *const u8, len as usize)
    };
    Ok(vec)
}
