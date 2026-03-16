// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// iOS platform bridge via objc2.
//
// Requires compilation with the iOS SDK (Xcode). Each trait method wraps the
// corresponding UIKit / Security.framework API through Objective-C message sends.
//
// This module is cfg-gated to `target_os = "ios"` and will not compile on other
// platforms.  All UIKit interactions require the main thread; methods that
// present view controllers will return `PresswerkError::Bridge` if called
// off-main.
//
// ## ABI Safety (src/abi/Bridge.idr)
//
// Unsafe code in this module falls into three categories, each covered by
// formal proofs in Bridge.idr:
//
// 1. **Toll-free bridging** (nsstr_as_obj, nsdata_as_obj, dict_as_cf):
//    Casts between NSString↔CFString, NSData↔CFData, NSDictionary↔CFDictionary.
//    Proven safe by Bridge.idr TollFreePair — identical size and alignment.
//
// 2. **ObjC message sends** (msg_send!, define_class! #[unsafe(...)]):
//    Required by the objc2 runtime. Selector correctness is verified by
//    Apple's SDK headers. Thread safety proven by Bridge.idr threadReq.
//
// 3. **Security.framework C FFI** (SecItemAdd, SecItemCopyMatching, etc.):
//    C function calls with toll-free bridged dictionary arguments.
//    Keychain semantics proven by Bridge.idr KeychainProperty.

#![cfg(target_os = "ios")]

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::mpsc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, ProtocolObject};
use objc2::{AllocAnyThread, MainThreadMarker, define_class, msg_send};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString, NSURL};
use objc2_ui_kit::{
    UIActivityViewController, UIApplication, UIDocumentPickerDelegate,
    UIDocumentPickerViewController, UIImagePickerController, UIImagePickerControllerDelegate,
    UIImagePickerControllerSourceType, UINavigationControllerDelegate,
    UIPrintInteractionController, UIViewController,
};

use presswerk_core::error::{PresswerkError, Result};

use crate::traits::*;

// ---------------------------------------------------------------------------
// Security.framework FFI (keychain)
// ---------------------------------------------------------------------------
// Security.framework is a C API not wrapped by objc2.  NSDictionary and
// CFDictionary are toll-free bridged, so we cast freely between them.

/// OSStatus success.
const ERR_SEC_SUCCESS: i32 = 0;
/// The item was not found in the keychain.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
/// A duplicate item already exists.
const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;

extern "C" {
    fn SecItemAdd(attributes: *const c_void, result: *mut *const c_void) -> i32;
    fn SecItemCopyMatching(query: *const c_void, result: *mut *const c_void) -> i32;
    fn SecItemUpdate(query: *const c_void, attrs_to_update: *const c_void) -> i32;
    fn SecItemDelete(query: *const c_void) -> i32;
}

// Security.framework constant strings.  These are `CFStringRef` globals,
// toll-free bridged with `NSString *`.  They are linked automatically when
// building against the iOS SDK.
extern "C" {
    static kSecClass: &'static NSString;
    static kSecClassGenericPassword: &'static NSString;
    static kSecAttrAccount: &'static NSString;
    static kSecAttrService: &'static NSString;
    static kSecValueData: &'static NSString;
    static kSecReturnData: &'static NSString;
    static kSecMatchLimit: &'static NSString;
    static kSecMatchLimitOne: &'static NSString;
}

/// The keychain service identifier for all Presswerk secrets.
const KEYCHAIN_SERVICE: &str = "org.hyperpolymath.presswerk";

// ---------------------------------------------------------------------------
// UIKit C functions & constants
// ---------------------------------------------------------------------------

// UIImagePickerControllerSourceType constants are provided by objc2-ui-kit.
// We use the Camera variant for capture_image().

extern "C" {
    /// Key into the `info` dictionary passed to the image-picker delegate.
    /// The value is the original `UIImage` chosen by the user.
    static UIImagePickerControllerOriginalImage: &'static NSString;

    /// Convert a `UIImage` to JPEG `NSData`.
    ///
    /// ```c
    /// NSData * _Nullable UIImageJPEGRepresentation(UIImage *image,
    ///                                              CGFloat compressionQuality);
    /// ```
    fn UIImageJPEGRepresentation(
        image: *const AnyObject,
        compression_quality: f64,
    ) -> *mut AnyObject;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Obtain the root `UIViewController` from the key window.
///
/// Uses the deprecated `keyWindow` property for broad iOS-version compat.
/// On iOS 15+ the caller should ideally walk `connectedScenes`, but for an
/// MVP bridge this is sufficient.
fn root_view_controller() -> Result<Retained<UIViewController>> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| PresswerkError::Bridge("must be called from the main thread".into()))?;

    let app = UIApplication::sharedApplication(mtm);

    // SAFETY: msg_send! to well-known UIApplication selectors (keyWindow,
    // rootViewController). MainThreadMarker guarantees we are on the main
    // thread (Bridge.idr threadReq = MainThread for UI operations).
    let root: Option<Retained<UIViewController>> = unsafe {
        let window: Option<Retained<AnyObject>> = msg_send![&app, keyWindow];
        window.and_then(|w| msg_send![&w, rootViewController])
    };

    root.ok_or_else(|| PresswerkError::Bridge("no root view controller available".into()))
}

/// Assert that we are on the main thread and return the marker.
fn require_main_thread() -> Result<MainThreadMarker> {
    MainThreadMarker::new()
        .ok_or_else(|| PresswerkError::Bridge("must be called from the main thread".into()))
}

/// Cast `NSDictionary` to a `*const c_void` for Security.framework calls.
///
/// NSDictionary and CFDictionary are toll-free bridged so this cast is valid.
fn dict_as_cf(dict: &NSDictionary<NSString, AnyObject>) -> *const c_void {
    dict as *const NSDictionary<NSString, AnyObject> as *const c_void
}

/// Cast a `*const NSString` to `*const AnyObject` (NSString *is* an
/// AnyObject).
///
/// SAFETY: NSString is a subclass of NSObject (which is AnyObject in objc2).
/// The pointer representation is identical — no layout change.
/// Proven by Bridge.idr TollFreePair: sameSize, sameAlign.
unsafe fn nsstr_as_obj(s: &NSString) -> &AnyObject {
    &*(s as *const NSString as *const AnyObject)
}

/// Cast an `NSData` reference to `&AnyObject`.
///
/// SAFETY: NSData is a subclass of NSObject. Same pointer, same layout.
/// Proven by Bridge.idr TollFreePair.
unsafe fn nsdata_as_obj(d: &NSData) -> &AnyObject {
    &*(d as *const NSData as *const AnyObject)
}

// ---------------------------------------------------------------------------
// Camera delegate (UIImagePickerControllerDelegate)
// ---------------------------------------------------------------------------
// Captures an `mpsc::Sender` so that `capture_image` can block until the
// user takes a photo or cancels.

struct CameraDelegateIvars {
    /// Channel sender; taken (`Option::take`) on first callback to prevent
    /// double-sends.
    sender: RefCell<Option<mpsc::Sender<Option<Vec<u8>>>>>,
}

// SAFETY: define_class! #[unsafe(super(NSObject))] declares CameraDelegate as
// an ObjC class inheriting from NSObject. This is required by objc2 for all
// custom ObjC classes. MainThreadOnly ensures delegate callbacks only fire on
// the main thread (Bridge.idr threadReq CaptureImage = MainThread).
define_class! {
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PresswerkCameraDelegate"]
    #[ivars = CameraDelegateIvars]
    struct CameraDelegate;

    unsafe impl UIImagePickerControllerDelegate for CameraDelegate {
        /// Called when the user has taken or chosen an image.
        #[unsafe(method(imagePickerController:didFinishPickingMediaWithInfo:))]
        fn did_finish(
            &self,
            picker: &UIImagePickerController,
            info: &NSDictionary<NSString, AnyObject>,
        ) {
            // SAFETY: objectForKey with UIImagePickerControllerOriginalImage
            // (extern static from UIKit). Returns nil if key not present.
            let image_bytes: Option<Vec<u8>> = unsafe {
                info.objectForKey(UIImagePickerControllerOriginalImage)
            }
            .and_then(|ui_image: Retained<AnyObject>| {
                // SAFETY: UIImageJPEGRepresentation is a UIKit C function.
                // Returns autoreleased NSData* (nil on failure).
                let raw = unsafe {
                    UIImageJPEGRepresentation(
                        &*ui_image as *const AnyObject,
                        0.9, // 90% JPEG quality
                    )
                };
                if raw.is_null() {
                    None
                } else {
                    // SAFETY: non-null result is an NSData* (toll-free bridged
                    // with CFData — Bridge.idr TollFreePair). We copy bytes
                    // immediately so the autorelease is harmless.
                    let ns_data: &NSData = unsafe { &*(raw as *const NSData) };
                    Some(ns_data.to_vec())
                }
            });

            // SAFETY: dismissViewControllerAnimated:completion: is a standard
            // UIViewController selector. Called on main thread (delegate is
            // MainThreadOnly).
            unsafe {
                let _: () = msg_send![
                    picker,
                    dismissViewControllerAnimated: true,
                    completion: std::ptr::null::<c_void>()
                ];
            }

            // Send the result through the channel.
            if let Some(tx) = self.ivars().sender.borrow_mut().take() {
                let _ = tx.send(image_bytes);
            }
        }

        /// Called when the user cancels the camera.
        #[unsafe(method(imagePickerControllerDidCancel:))]
        fn did_cancel(&self, picker: &UIImagePickerController) {
            // SAFETY: dismissViewControllerAnimated:completion: — same as above.
            unsafe {
                let _: () = msg_send![
                    picker,
                    dismissViewControllerAnimated: true,
                    completion: std::ptr::null::<c_void>()
                ];
            }
            if let Some(tx) = self.ivars().sender.borrow_mut().take() {
                let _ = tx.send(None);
            }
        }
    }

    // UIImagePickerController requires its delegate to also conform to
    // UINavigationControllerDelegate.  We provide an empty impl.
    unsafe impl UINavigationControllerDelegate for CameraDelegate {}
}

impl CameraDelegate {
    /// Create a new camera delegate wired to `tx`.
    fn new(mtm: MainThreadMarker, tx: mpsc::Sender<Option<Vec<u8>>>) -> Retained<Self> {
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(CameraDelegateIvars {
            sender: RefCell::new(Some(tx)),
        });
        // SAFETY: Standard NSObject init via super. The alloc above provides
        // a valid, allocated-but-uninitialised object; init completes it.
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------------------------------------------------------------------
// Document picker delegate (UIDocumentPickerDelegate)
// ---------------------------------------------------------------------------

struct DocPickerDelegateIvars {
    sender: RefCell<Option<mpsc::Sender<Option<String>>>>,
}

// SAFETY: define_class! #[unsafe(super(NSObject))] declares DocPickerDelegate
// as an ObjC class inheriting from NSObject. MainThreadOnly ensures delegate
// callbacks fire on the main thread (Bridge.idr threadReq PickFile = MainThread).
define_class! {
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PresswerkDocPickerDelegate"]
    #[ivars = DocPickerDelegateIvars]
    struct DocPickerDelegate;

    unsafe impl UIDocumentPickerDelegate for DocPickerDelegate {
        /// Called when the user selects one or more documents.
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn did_pick(
            &self,
            _controller: &UIDocumentPickerViewController,
            urls: &NSArray<NSURL>,
        ) {
            // Take the first selected URL and convert to a file-system path.
            let path: Option<String> = urls.firstObject().and_then(|url| {
                // SAFETY: msg_send to NSURL.path property — well-known
                // Foundation selector, returns NSString? for the file path.
                let ns_path: Option<Retained<NSString>> =
                    unsafe { msg_send![&url, path] };
                ns_path.map(|p| p.to_string())
            });
            if let Some(tx) = self.ivars().sender.borrow_mut().take() {
                let _ = tx.send(path);
            }
        }

        /// Called when the user cancels the document picker.
        #[unsafe(method(documentPickerWasCancelled:))]
        fn was_cancelled(&self, _controller: &UIDocumentPickerViewController) {
            if let Some(tx) = self.ivars().sender.borrow_mut().take() {
                let _ = tx.send(None);
            }
        }
    }
}

impl DocPickerDelegate {
    fn new(mtm: MainThreadMarker, tx: mpsc::Sender<Option<String>>) -> Retained<Self> {
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(DocPickerDelegateIvars {
            sender: RefCell::new(Some(tx)),
        });
        // SAFETY: Standard NSObject init via super (same as CameraDelegate::new).
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------------------------------------------------------------------
// IosBridge
// ---------------------------------------------------------------------------

/// Concrete iOS platform bridge.
///
/// All methods that present UI controllers require invocation from the main
/// thread.  The keychain methods (`NativeKeychain`) are thread-safe and may
/// be called from any thread.
pub struct IosBridge;

impl IosBridge {
    /// Create a new iOS bridge instance.
    pub fn new() -> Self {
        Self
    }
}

impl PlatformBridge for IosBridge {
    fn platform_name(&self) -> &str {
        "iOS"
    }
}

// ---------------------------------------------------------------------------
// NativePrint -- UIPrintInteractionController
// ---------------------------------------------------------------------------

impl NativePrint for IosBridge {
    /// Present the system print dialog for the supplied document bytes.
    ///
    /// The `mime_type` parameter is informational; the print system infers
    /// the document type from the raw data.  This is fire-and-forget: once
    /// the dialog is presented the user drives the rest of the interaction.
    ///
    /// # Errors
    ///
    /// Returns `PresswerkError::Bridge` if not called from the main thread
    /// or if the print controller refuses to present.
    fn show_print_dialog(&self, document: &[u8], _mime_type: &str) -> Result<()> {
        let mtm = require_main_thread()?;

        tracing::info!(
            bytes = document.len(),
            "iOS: presenting UIPrintInteractionController"
        );

        let controller = UIPrintInteractionController::sharedPrintController(mtm);
        let ns_data = NSData::with_bytes(document);

        // SAFETY: setPrintingItem is a well-known UIPrintInteractionController
        // selector. MainThreadMarker (above) guarantees main-thread execution
        // (Bridge.idr threadReq ShowPrintDialog = MainThread).
        unsafe {
            controller.setPrintingItem(Some(&ns_data));
        }

        // SAFETY: presentAnimated_completionHandler is a documented UIKit method.
        // Main-thread requirement satisfied by require_main_thread() above.
        let presented = unsafe { controller.presentAnimated_completionHandler(true, None) };

        if presented {
            Ok(())
        } else {
            Err(PresswerkError::Bridge(
                "UIPrintInteractionController refused to present".into(),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// NativeCamera -- UIImagePickerController
// ---------------------------------------------------------------------------

impl NativeCamera for IosBridge {
    /// Launch the device camera and return captured JPEG bytes.
    ///
    /// This method **must** be called from the main thread.  It blocks the
    /// current thread until the user either takes a photo (returns
    /// `Ok(Some(jpeg_bytes))`) or cancels (`Ok(None)`).
    ///
    /// The returned bytes are JPEG-encoded at 90 % quality.
    ///
    /// # Errors
    ///
    /// Returns `PresswerkError::Bridge` when:
    /// - Called off the main thread.
    /// - The camera source type is unavailable (e.g. Simulator).
    /// - No root view controller is available for presentation.
    fn capture_image(&self) -> Result<Option<Vec<u8>>> {
        let mtm = require_main_thread()?;

        tracing::info!("iOS: launching UIImagePickerController for camera");

        // Verify camera availability.
        let available = UIImagePickerController::isSourceTypeAvailable(
            UIImagePickerControllerSourceType::Camera,
            mtm,
        );
        if !available {
            return Err(PresswerkError::Bridge(
                "camera source type is not available on this device".into(),
            ));
        }

        let picker = UIImagePickerController::new(mtm);
        // SAFETY: setSourceType is a UIImagePickerController property setter.
        // We verified availability with isSourceTypeAvailable above.
        unsafe {
            picker.setSourceType(UIImagePickerControllerSourceType::Camera);
        }

        // Channel for the delegate to deliver the result.
        let (tx, rx) = mpsc::channel();
        let delegate = CameraDelegate::new(mtm, tx);

        // SAFETY: CameraDelegate conforms to both UIImagePickerControllerDelegate
        // and UINavigationControllerDelegate (defined via define_class! above).
        // The pointer cast CameraDelegate→AnyObject is safe: CameraDelegate is an
        // NSObject subclass with identical pointer representation.
        unsafe {
            let delegate_obj: &AnyObject =
                &*((&*delegate) as *const CameraDelegate as *const AnyObject);
            picker.setDelegate(Some(delegate_obj));
        }

        // Present modally on the root view controller.
        let root_vc = root_view_controller()?;
        // SAFETY: presentViewController is a UIViewController method.
        // Main-thread requirement satisfied by require_main_thread() above
        // (Bridge.idr threadReq CaptureImage = MainThread).
        unsafe {
            root_vc.presentViewController_animated_completion(&picker, true, None);
        }

        // Block until the delegate fires.  The main run loop continues to
        // pump while the picker is presented, so the delegate callbacks
        // will execute on the main thread as expected.
        let result = rx
            .recv()
            .map_err(|e| PresswerkError::Bridge(format!("camera delegate channel error: {e}")))?;

        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// NativeFilePicker -- UIDocumentPickerViewController
// ---------------------------------------------------------------------------

impl NativeFilePicker for IosBridge {
    /// Present a document picker filtered to the given MIME types.
    ///
    /// MIME types are converted to `UTType` identifiers via
    /// `[UTType typeWithMIMEType:]`.  Unrecognised types are silently
    /// dropped; if none resolve the picker defaults to `UTType.data`
    /// (public.data), which matches all file types.
    ///
    /// Returns the file-system path of the selected document, or `None` if
    /// the user cancelled.
    ///
    /// # Errors
    ///
    /// Returns `PresswerkError::Bridge` if not called from the main thread.
    fn pick_file(&self, mime_types: &[&str]) -> Result<Option<String>> {
        let mtm = require_main_thread()?;

        tracing::info!(
            types = ?mime_types,
            "iOS: presenting UIDocumentPickerViewController"
        );

        // Convert MIME types to UTType objects via the ObjC runtime.
        // UTType lives in UniformTypeIdentifiers.framework, which is
        // linked automatically on iOS 14+.
        let ut_types: Vec<Retained<AnyObject>> = mime_types
            .iter()
            .filter_map(|mime| {
                let ns_mime = NSString::from_str(mime);
                // SAFETY: msg_send to UTType class method (UniformTypeIdentifiers.framework).
                // Returns nil for unrecognised MIME types, which filter_map discards.
                let ut: Option<Retained<AnyObject>> = unsafe {
                    msg_send![
                        objc2::class!(UTType),
                        typeWithMIMEType: &*ns_mime
                    ]
                };
                ut
            })
            .collect();

        // Fall back to UTType.data (public.data) if nothing resolved.
        let content_types: Retained<NSArray<AnyObject>> = if ut_types.is_empty() {
            // SAFETY: msg_send to UTType class property. Returns the well-known
            // public.data UTType — always non-nil.
            let public_data: Retained<AnyObject> =
                unsafe { msg_send![objc2::class!(UTType), dataType] };
            NSArray::from_retained_slice(&[public_data])
        } else {
            NSArray::from_retained_slice(&ut_types)
        };

        // SAFETY: ObjC alloc+init pattern for UIDocumentPickerViewController.
        // initForOpeningContentTypes: takes NSArray<UTType>; we pass NSArray<AnyObject>
        // which is layout-compatible at the ObjC level (type erasure).
        let picker: Retained<UIDocumentPickerViewController> = unsafe {
            let alloc: Retained<UIDocumentPickerViewController> =
                msg_send![objc2::class!(UIDocumentPickerViewController), alloc];
            msg_send![
                alloc,
                initForOpeningContentTypes: &*content_types
            ]
        };

        // Wire up the delegate.
        let (tx, rx) = mpsc::channel();
        let delegate = DocPickerDelegate::new(mtm, tx);

        // SAFETY: DocPickerDelegate conforms to UIDocumentPickerDelegate
        // (defined via define_class! above). ProtocolObject::from_ref is the
        // type-safe way to pass the delegate.
        unsafe {
            picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        }

        // Present on the root view controller.
        let root_vc = root_view_controller()?;
        // SAFETY: presentViewController is a UIViewController method.
        // Main-thread satisfied by require_main_thread() above
        // (Bridge.idr threadReq PickFile = MainThread).
        unsafe {
            root_vc.presentViewController_animated_completion(&picker, true, None);
        }

        let result = rx
            .recv()
            .map_err(|e| PresswerkError::Bridge(format!("document picker channel error: {e}")))?;

        Ok(result)
    }

    /// Read the bytes of a previously picked file.
    ///
    /// Uses `std::fs::read` which works for paths within the app sandbox and
    /// for files the user has granted access to via the document picker (the
    /// security-scoped bookmark is resolved at pick time).
    ///
    /// For files outside the sandbox, the caller should start a
    /// security-scoped access session before calling this method.
    fn read_picked_file(&self, path: &str) -> Result<Vec<u8>> {
        tracing::debug!(path, "iOS: reading picked file");
        std::fs::read(path)
            .map_err(|e| PresswerkError::Bridge(format!("failed to read picked file: {e}")))
    }
}

// ---------------------------------------------------------------------------
// NativeKeychain -- Security.framework
// ---------------------------------------------------------------------------

impl NativeKeychain for IosBridge {
    /// Store `value` in the iOS Keychain under `key`.
    ///
    /// If an entry already exists for `key` it is updated in place via
    /// `SecItemUpdate`.
    ///
    /// This method is thread-safe and does not require the main thread.
    fn store_secret(&self, key: &str, value: &[u8]) -> Result<()> {
        tracing::info!(key, "iOS: storing secret in Keychain");

        let ns_key = NSString::from_str(key);
        let ns_service = NSString::from_str(KEYCHAIN_SERVICE);
        let ns_data = NSData::with_bytes(value);

        // SAFETY: Accessing extern statics from Security.framework. These are
        // constant CFStringRef values linked by the iOS SDK, valid for process lifetime.
        let keys: Vec<&NSString> =
            unsafe { vec![kSecClass, kSecAttrAccount, kSecAttrService, kSecValueData] };
        // SAFETY: nsstr_as_obj/nsdata_as_obj are toll-free bridge casts.
        // Proven safe by Bridge.idr TollFreePair.
        let values: Vec<&AnyObject> = unsafe {
            vec![
                nsstr_as_obj(kSecClassGenericPassword),
                nsstr_as_obj(&ns_key),
                nsstr_as_obj(&ns_service),
                nsdata_as_obj(&ns_data),
            ]
        };

        let dict = NSDictionary::from_slices(&keys, &values);

        // SAFETY: dict_as_cf casts NSDictionary to CFDictionary (toll-free bridged).
        // SecItemAdd is a C function from Security.framework with well-defined semantics.
        // Bridge.idr KeychainProperty StoreLoad proves store-then-load consistency.
        let status = unsafe { SecItemAdd(dict_as_cf(&dict), std::ptr::null_mut()) };

        match status {
            ERR_SEC_SUCCESS => Ok(()),
            ERR_SEC_DUPLICATE_ITEM => {
                // Item exists -- update it instead.
                self.update_secret(key, value)
            }
            code => Err(PresswerkError::Bridge(format!(
                "SecItemAdd failed with OSStatus {code}"
            ))),
        }
    }

    /// Retrieve a secret from the iOS Keychain by `key`.
    ///
    /// Returns `Ok(None)` if no entry exists for the given key.
    ///
    /// This method is thread-safe.
    fn load_secret(&self, key: &str) -> Result<Option<Vec<u8>>> {
        tracing::debug!(key, "iOS: loading secret from Keychain");

        let ns_key = NSString::from_str(key);
        let ns_service = NSString::from_str(KEYCHAIN_SERVICE);

        // kSecReturnData expects a CFBoolean.  kCFBooleanTrue is toll-free
        // bridged with `[NSNumber numberWithBool:YES]`.
        // SAFETY: msg_send to NSNumber class method. Returns a valid retained object.
        let cf_true: Retained<AnyObject> =
            unsafe { msg_send![objc2::class!(NSNumber), numberWithBool: Bool::YES] };

        // SAFETY: Accessing Security.framework extern statics (process-lifetime constants).
        let keys: Vec<&NSString> = unsafe {
            vec![
                kSecClass,
                kSecAttrAccount,
                kSecAttrService,
                kSecReturnData,
                kSecMatchLimit,
            ]
        };
        // SAFETY: Toll-free bridge casts (Bridge.idr TollFreePair).
        let values: Vec<&AnyObject> = unsafe {
            vec![
                nsstr_as_obj(kSecClassGenericPassword),
                nsstr_as_obj(&ns_key),
                nsstr_as_obj(&ns_service),
                &*cf_true,
                nsstr_as_obj(kSecMatchLimitOne),
            ]
        };

        let dict = NSDictionary::from_slices(&keys, &values);

        let mut result: *const c_void = std::ptr::null();
        // SAFETY: SecItemCopyMatching is a Security.framework C function.
        // dict_as_cf is a toll-free bridge cast (Bridge.idr TollFreePair).
        // On success, `result` receives a retained CFData (toll-free bridged with NSData).
        let status = unsafe { SecItemCopyMatching(dict_as_cf(&dict), &mut result) };

        match status {
            ERR_SEC_SUCCESS => {
                if result.is_null() {
                    return Ok(None);
                }
                // SAFETY: `result` is a retained CFData. CFData and NSData are
                // toll-free bridged (Bridge.idr TollFreePair) — identical layout.
                let ns_data: &NSData = unsafe { &*(result as *const NSData) };
                let bytes = ns_data.to_vec();

                // SAFETY: Balance the implicit +1 retain from SecItemCopyMatching.
                // We own this reference and must release it.
                unsafe {
                    let _: () = msg_send![result as *const AnyObject, release];
                }

                Ok(Some(bytes))
            }
            ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            code => Err(PresswerkError::Bridge(format!(
                "SecItemCopyMatching failed with OSStatus {code}"
            ))),
        }
    }

    /// Delete a secret from the iOS Keychain.
    ///
    /// Silently succeeds if no entry exists for `key`.
    ///
    /// This method is thread-safe.
    fn delete_secret(&self, key: &str) -> Result<()> {
        tracing::info!(key, "iOS: deleting secret from Keychain");

        let ns_key = NSString::from_str(key);
        let ns_service = NSString::from_str(KEYCHAIN_SERVICE);

        // SAFETY: Security.framework extern statics (process-lifetime constants).
        let keys: Vec<&NSString> = unsafe { vec![kSecClass, kSecAttrAccount, kSecAttrService] };
        // SAFETY: Toll-free bridge casts (Bridge.idr TollFreePair).
        let values: Vec<&AnyObject> = unsafe {
            vec![
                nsstr_as_obj(kSecClassGenericPassword),
                nsstr_as_obj(&ns_key),
                nsstr_as_obj(&ns_service),
            ]
        };

        let dict = NSDictionary::from_slices(&keys, &values);
        // SAFETY: SecItemDelete C FFI with toll-free bridged dict.
        // Bridge.idr KeychainProperty DeleteLoad proves delete-then-load = Nothing.
        let status = unsafe { SecItemDelete(dict_as_cf(&dict)) };

        match status {
            ERR_SEC_SUCCESS | ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            code => Err(PresswerkError::Bridge(format!(
                "SecItemDelete failed with OSStatus {code}"
            ))),
        }
    }
}

/// Private keychain helpers.
impl IosBridge {
    /// Update an existing keychain entry with new value bytes.
    fn update_secret(&self, key: &str, value: &[u8]) -> Result<()> {
        let ns_key = NSString::from_str(key);
        let ns_service = NSString::from_str(KEYCHAIN_SERVICE);
        let ns_data = NSData::with_bytes(value);

        // SAFETY: Security.framework extern statics (process-lifetime constants).
        let query_keys: Vec<&NSString> =
            unsafe { vec![kSecClass, kSecAttrAccount, kSecAttrService] };
        // SAFETY: Toll-free bridge casts (Bridge.idr TollFreePair).
        let query_values: Vec<&AnyObject> = unsafe {
            vec![
                nsstr_as_obj(kSecClassGenericPassword),
                nsstr_as_obj(&ns_key),
                nsstr_as_obj(&ns_service),
            ]
        };
        let query = NSDictionary::from_slices(&query_keys, &query_values);

        // SAFETY: Security.framework extern static (process-lifetime constant).
        let update_keys: Vec<&NSString> = unsafe { vec![kSecValueData] };
        // SAFETY: nsdata_as_obj is a toll-free bridge cast (Bridge.idr TollFreePair).
        let update_values: Vec<&AnyObject> = unsafe { vec![nsdata_as_obj(&ns_data)] };
        let update = NSDictionary::from_slices(&update_keys, &update_values);

        // SAFETY: SecItemUpdate is a Security.framework C function.
        // dict_as_cf casts NSDictionary→CFDictionary (toll-free bridged).
        // Bridge.idr KeychainProperty LastWriteWins proves update semantics.
        let status = unsafe { SecItemUpdate(dict_as_cf(&query), dict_as_cf(&update)) };

        if status == ERR_SEC_SUCCESS {
            Ok(())
        } else {
            Err(PresswerkError::Bridge(format!(
                "SecItemUpdate failed with OSStatus {status}"
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// NativeShare -- UIActivityViewController
// ---------------------------------------------------------------------------

impl NativeShare for IosBridge {
    /// Present the iOS share sheet for the file at `path`.
    ///
    /// The `mime_type` parameter is currently unused; the share sheet infers
    /// the content type from the file extension / UTI.
    ///
    /// # Errors
    ///
    /// Returns `PresswerkError::Bridge` if not called from the main thread
    /// or if no root view controller is available.
    fn share_file(&self, path: &str, _mime_type: &str) -> Result<()> {
        let _mtm = require_main_thread()?;

        tracing::info!(path, "iOS: presenting UIActivityViewController");

        let ns_path = NSString::from_str(path);
        let url = NSURL::fileURLWithPath(&ns_path);

        // UIActivityViewController expects an NSArray of activity items.
        // We upcast NSURL -> AnyObject via Retained::into_super.
        let url_as_obj: Retained<AnyObject> = Retained::into_super(Retained::into_super(url));
        let items = NSArray::from_retained_slice(&[url_as_obj]);

        // SAFETY: ObjC alloc+init pattern for UIActivityViewController.
        // initWithActivityItems:applicationActivities: takes NSArray of activity
        // items and optional NSArray of UIActivity objects (nil = system default).
        let activity_vc: Retained<UIActivityViewController> = unsafe {
            let alloc: Retained<UIActivityViewController> =
                msg_send![objc2::class!(UIActivityViewController), alloc];
            msg_send![
                alloc,
                initWithActivityItems: &*items,
                applicationActivities: std::ptr::null::<AnyObject>()
            ]
        };

        let root_vc = root_view_controller()?;
        // SAFETY: presentViewController is a UIViewController method.
        // Main-thread satisfied by require_main_thread() above
        // (Bridge.idr threadReq ShareFile = MainThread).
        unsafe {
            root_vc.presentViewController_animated_completion(&activity_vc, true, None);
        }

        Ok(())
    }

    /// Share text content via the iOS share sheet.
    fn share_text(&self, text: &str) -> Result<()> {
        let _mtm = require_main_thread()?;

        tracing::info!("iOS: sharing text via UIActivityViewController");

        let ns_text = NSString::from_str(text);
        let text_as_obj: Retained<AnyObject> = Retained::into_super(Retained::into_super(ns_text));
        let items = NSArray::from_retained_slice(&[text_as_obj]);

        // SAFETY: Same pattern as share_file — UIActivityViewController alloc+init.
        let activity_vc: Retained<UIActivityViewController> = unsafe {
            let alloc: Retained<UIActivityViewController> =
                msg_send![objc2::class!(UIActivityViewController), alloc];
            msg_send![
                alloc,
                initWithActivityItems: &*items,
                applicationActivities: std::ptr::null::<AnyObject>()
            ]
        };

        let root_vc = root_view_controller()?;
        // SAFETY: presentViewController — main thread confirmed above.
        unsafe {
            root_vc.presentViewController_animated_completion(&activity_vc, true, None);
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// Stub implementations for connection types not yet wired to iOS APIs
// ---------------------------------------------------------------------------

impl NativeUsbPrint for IosBridge {
    fn detect_usb_printers(&self) -> Result<Vec<UsbPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_usb(&self, _device_id: &str, _document: &[u8], _mime_type: &str) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeBluetoothPrint for IosBridge {
    fn scan_bluetooth_printers(&self) -> Result<Vec<BluetoothPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_bluetooth(&self, _device_id: &str, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeNfcPrint for IosBridge {
    fn read_nfc_printer_tag(&self) -> Result<Option<NfcPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeConnectivity for IosBridge {
    fn wifi_ssid(&self) -> Result<Option<String>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn supports_wifi_direct(&self) -> bool {
        false
    }

    fn discover_wifi_direct_printers(&self) -> Result<Vec<WifiDirectPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeFireWirePrint for IosBridge {
    fn detect_firewire_printers(&self) -> Result<Vec<FireWirePrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_firewire(&self, _device_id: &str, _document: &[u8], _mime_type: &str) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeLightningPrint for IosBridge {
    fn detect_lightning_printers(&self) -> Result<Vec<LightningPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_lightning(&self, _device_id: &str, _document: &[u8], _mime_type: &str) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeThunderboltPrint for IosBridge {
    fn detect_thunderbolt_printers(&self) -> Result<Vec<ThunderboltPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_thunderbolt(&self, _device_id: &str, _document: &[u8], _mime_type: &str) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeSerialPrint for IosBridge {
    fn detect_serial_printers(&self) -> Result<Vec<SerialPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_serial(&self, _port: &str, _baud_rate: u32, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeParallelPrint for IosBridge {
    fn detect_parallel_printers(&self) -> Result<Vec<ParallelPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_parallel(&self, _port: &str, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeInfraredPrint for IosBridge {
    fn scan_infrared_printers(&self) -> Result<Vec<InfraredPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_infrared(&self, _device_id: &str, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeIBeaconDiscover for IosBridge {
    fn scan_ibeacon_printers(&self) -> Result<Vec<IBeaconPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeLiFiPrint for IosBridge {
    fn detect_lifi_endpoints(&self) -> Result<Vec<LiFiEndpointInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_lifi(&self, _endpoint_id: &str, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeUsbDrivePrint for IosBridge {
    fn detect_usb_drives(&self) -> Result<Vec<UsbDriveInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn copy_to_usb_drive(&self, _drive_id: &str, _document: &[u8], _filename: &str) -> Result<String> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the bridge reports the correct platform name.
    #[test]
    fn platform_name() {
        let bridge = IosBridge::new();
        assert_eq!(bridge.platform_name(), "iOS");
    }

    // Integration tests for UI-presenting methods require a running iOS app
    // with a key window.  They are exercised in the Xcode test target rather
    // than via `cargo test`.
}
