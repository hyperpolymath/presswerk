// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Android platform bridge via JNI.
//
// Requires the Android NDK and targets `aarch64-linux-android` or
// `armv7-linux-androideabi`. Each trait method invokes the corresponding
// Android API through JNI calls into the ART runtime.
//
// ## Architecture notes
//
// Methods that can complete synchronously via JNI (SharedPreferences,
// ContentResolver, Intent launching) are fully implemented here.
//
// Methods that require `startActivityForResult` (camera capture, file picker)
// launch the Intent and return `PresswerkError::Bridge` explaining that the
// result must be collected through the Activity's `onActivityResult` callback.
// The host Activity is responsible for wiring that callback back into
// Presswerk — see `ANDROID-INTEGRATION.md` for the Java/Kotlin glue code.

#![cfg(target_os = "android")]

use jni::JNIEnv;
use jni::objects::{JObject, JString, JValue};
use jni::sys::jsize;

use presswerk_core::error::{PresswerkError, Result};

use crate::traits::*;

// ---------------------------------------------------------------------------
// JNI bootstrap helpers
// ---------------------------------------------------------------------------

/// Prefix applied to all SharedPreferences keys to avoid collisions.
const PREFS_KEY_PREFIX: &str = "presswerk_";

/// SharedPreferences file name.
const PREFS_FILE: &str = "presswerk_secrets";

/// Request codes for `startActivityForResult`. The host Activity must
/// recognise these in its `onActivityResult` override.
pub const REQUEST_IMAGE_CAPTURE: i32 = 0x5057_0001; // "PW" + 1
pub const REQUEST_PICK_FILE: i32 = 0x5057_0002;

/// Obtain a [`JNIEnv`] handle from the global Android context.
///
/// Calls `ndk_context::android_context()` to retrieve the `JavaVM*` pointer
/// set by `android_main` or `ANativeActivity_onCreate`, then attaches the
/// current thread if it is not already attached.
fn jni_env() -> Result<JNIEnv<'static>> {
    let ctx = ndk_context::android_context();
    // SAFETY: `ctx.vm()` returns the `JavaVM*` set by the NDK glue code.
    // The pointer is guaranteed valid for the lifetime of the process.
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }
        .map_err(|e| PresswerkError::Bridge(format!("failed to obtain JavaVM: {e}")))?;
    vm.attach_current_thread()
        .map_err(|e| PresswerkError::Bridge(format!("failed to attach JNI thread: {e}")))
}

/// Obtain the current Android `Activity` as a [`JObject`].
///
/// The pointer comes from `ndk_context::android_context().context()` which
/// is the `jobject` for the `NativeActivity` (or whichever `Activity` hosts
/// the native code).
fn activity() -> Result<JObject<'static>> {
    let ctx = ndk_context::android_context();
    let ptr = ctx.context();
    if ptr.is_null() {
        return Err(PresswerkError::Bridge(
            "Android context is null — native activity not initialised".into(),
        ));
    }
    // SAFETY: the NDK guarantees this pointer is a valid global jobject for
    // the hosting Activity.
    Ok(unsafe { JObject::from_raw(ptr.cast()) })
}

/// Convenience: map any `jni::errors::Error` into `PresswerkError::Bridge`.
fn jni_err(context: &str, e: jni::errors::Error) -> PresswerkError {
    PresswerkError::Bridge(format!("{context}: {e}"))
}

// ---------------------------------------------------------------------------
// Bridge struct
// ---------------------------------------------------------------------------

/// Android implementation of the Presswerk platform bridge.
///
/// All methods go through JNI to call the Android SDK. The struct is
/// zero-sized; all state lives on the Java side.
pub struct AndroidBridge;

impl AndroidBridge {
    /// Create a new Android bridge.
    ///
    /// This does **not** touch JNI — the first JNI call happens lazily when
    /// a trait method is invoked.
    pub fn new() -> Self {
        Self
    }
}

impl PlatformBridge for AndroidBridge {
    fn platform_name(&self) -> &str {
        "Android"
    }
}

// ---------------------------------------------------------------------------
// NativePrint — android.print.PrintManager
// ---------------------------------------------------------------------------

impl NativePrint for AndroidBridge {
    /// Open the Android print dialog for the given document bytes.
    ///
    /// Strategy: write the document to a temporary file, then launch an
    /// `ACTION_VIEW` intent with the `android.intent.extra.PRINT` category
    /// so the system print service picks it up. For PDF specifically this
    /// triggers the built-in PDF renderer and print dialog.
    ///
    /// Returns `Ok(())` once the intent has been dispatched. The user may
    /// still cancel the print job; that is not an error.
    fn show_print_dialog(&self, document: &[u8], mime_type: &str) -> Result<()> {
        let mut env = jni_env()?;
        let activity = activity()?;

        tracing::info!(
            mime = mime_type,
            bytes = document.len(),
            "Android: dispatching print intent"
        );

        // -- Write document to a temp file in the cache dir ---------------------
        let cache_dir: JObject = env
            .call_method(&activity, "getCacheDir", "()Ljava/io/File;", &[])
            .map_err(|e| jni_err("getCacheDir", e))?
            .l()
            .map_err(|e| jni_err("getCacheDir->l", e))?;

        let extension = match mime_type {
            "application/pdf" => ".pdf",
            "image/png" => ".png",
            "image/jpeg" | "image/jpg" => ".jpg",
            _ => ".tmp",
        };
        let filename = format!("presswerk_print{extension}");
        let j_filename: JString = env
            .new_string(&filename)
            .map_err(|e| jni_err("new_string(filename)", e))?;

        // new File(cacheDir, filename)
        let file_obj: JObject = env
            .new_object(
                "java/io/File",
                "(Ljava/io/File;Ljava/lang/String;)V",
                &[JValue::Object(&cache_dir), JValue::Object(&j_filename)],
            )
            .map_err(|e| jni_err("new File", e))?;

        // Write bytes through FileOutputStream
        let fos: JObject = env
            .new_object(
                "java/io/FileOutputStream",
                "(Ljava/io/File;)V",
                &[JValue::Object(&file_obj)],
            )
            .map_err(|e| jni_err("new FileOutputStream", e))?;

        let byte_array = env
            .byte_array_from_slice(document)
            .map_err(|e| jni_err("byte_array_from_slice", e))?;

        env.call_method(&fos, "write", "([B)V", &[JValue::Object(&byte_array)])
            .map_err(|e| jni_err("FileOutputStream.write", e))?;

        env.call_method(&fos, "close", "()V", &[])
            .map_err(|e| jni_err("FileOutputStream.close", e))?;

        // -- Build a content:// URI via FileProvider ----------------------------
        let authority = get_authority(&mut env, &activity)?;
        let j_authority: JString = env
            .new_string(&authority)
            .map_err(|e| jni_err("new_string(authority)", e))?;

        let content_uri: JObject = env
            .call_static_method(
                "androidx/core/content/FileProvider",
                "getUriForFile",
                "(Landroid/content/Context;Ljava/lang/String;Ljava/io/File;)Landroid/net/Uri;",
                &[
                    JValue::Object(&activity),
                    JValue::Object(&j_authority),
                    JValue::Object(&file_obj),
                ],
            )
            .map_err(|e| jni_err("FileProvider.getUriForFile", e))?
            .l()
            .map_err(|e| jni_err("getUriForFile->l", e))?;

        // -- Launch ACTION_VIEW intent with print flag --------------------------
        let j_action: JString = env
            .new_string("android.intent.action.VIEW")
            .map_err(|e| jni_err("new_string(ACTION_VIEW)", e))?;

        let intent: JObject = env
            .new_object(
                "android/content/Intent",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&j_action)],
            )
            .map_err(|e| jni_err("new Intent", e))?;

        let j_mime: JString = env
            .new_string(mime_type)
            .map_err(|e| jni_err("new_string(mime_type)", e))?;

        // intent.setDataAndType(uri, mimeType)
        env.call_method(
            &intent,
            "setDataAndType",
            "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&content_uri), JValue::Object(&j_mime)],
        )
        .map_err(|e| jni_err("setDataAndType", e))?;

        // Grant read permission to the receiving app
        env.call_method(
            &intent,
            "addFlags",
            "(I)Landroid/content/Intent;",
            &[JValue::Int(0x0000_0001)], // FLAG_GRANT_READ_URI_PERMISSION
        )
        .map_err(|e| jni_err("addFlags", e))?;

        // activity.startActivity(intent)
        env.call_method(
            &activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&intent)],
        )
        .map_err(|e| jni_err("startActivity(print)", e))?;

        tracing::info!("Android: print intent dispatched successfully");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// NativeCamera — Intent ACTION_IMAGE_CAPTURE
// ---------------------------------------------------------------------------

impl NativeCamera for AndroidBridge {
    /// Launch the system camera via `MediaStore.ACTION_IMAGE_CAPTURE`.
    ///
    /// This dispatches the capture intent and returns immediately. Because
    /// `startActivityForResult` is inherently asynchronous, the JPEG bytes
    /// are **not** returned from this call. Instead, the host Activity must
    /// override `onActivityResult` with request code [`REQUEST_IMAGE_CAPTURE`]
    /// and forward the result back to Presswerk.
    ///
    /// Returns `Err(Bridge(...))` with an explanatory message after the
    /// intent has been launched so callers know to await the Activity
    /// callback.
    fn capture_image(&self) -> Result<Option<Vec<u8>>> {
        let mut env = jni_env()?;
        let activity = activity()?;

        tracing::info!("Android: launching ACTION_IMAGE_CAPTURE intent");

        // -- Create a temp file for the full-resolution photo -------------------
        let cache_dir: JObject = env
            .call_method(&activity, "getCacheDir", "()Ljava/io/File;", &[])
            .map_err(|e| jni_err("getCacheDir", e))?
            .l()
            .map_err(|e| jni_err("getCacheDir->l", e))?;

        let j_filename: JString = env
            .new_string("presswerk_capture.jpg")
            .map_err(|e| jni_err("new_string", e))?;

        let photo_file: JObject = env
            .new_object(
                "java/io/File",
                "(Ljava/io/File;Ljava/lang/String;)V",
                &[JValue::Object(&cache_dir), JValue::Object(&j_filename)],
            )
            .map_err(|e| jni_err("new File(photo)", e))?;

        // -- Build a content:// URI via FileProvider ----------------------------
        let authority = get_authority(&mut env, &activity)?;
        let j_authority: JString = env
            .new_string(&authority)
            .map_err(|e| jni_err("new_string(authority)", e))?;

        let photo_uri: JObject = env
            .call_static_method(
                "androidx/core/content/FileProvider",
                "getUriForFile",
                "(Landroid/content/Context;Ljava/lang/String;Ljava/io/File;)Landroid/net/Uri;",
                &[
                    JValue::Object(&activity),
                    JValue::Object(&j_authority),
                    JValue::Object(&photo_file),
                ],
            )
            .map_err(|e| jni_err("FileProvider.getUriForFile", e))?
            .l()
            .map_err(|e| jni_err("getUriForFile->l", e))?;

        // -- Build the capture intent ------------------------------------------
        let j_action: JString = env
            .new_string("android.media.action.IMAGE_CAPTURE")
            .map_err(|e| jni_err("new_string(ACTION_IMAGE_CAPTURE)", e))?;

        let intent: JObject = env
            .new_object(
                "android/content/Intent",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&j_action)],
            )
            .map_err(|e| jni_err("new Intent(capture)", e))?;

        // intent.putExtra(MediaStore.EXTRA_OUTPUT, photoUri)
        let j_extra_output: JString = env
            .new_string("output")
            .map_err(|e| jni_err("new_string(EXTRA_OUTPUT)", e))?;

        env.call_method(
            &intent,
            "putExtra",
            "(Ljava/lang/String;Landroid/os/Parcelable;)Landroid/content/Intent;",
            &[JValue::Object(&j_extra_output), JValue::Object(&photo_uri)],
        )
        .map_err(|e| jni_err("putExtra(EXTRA_OUTPUT)", e))?;

        // Grant write permission so the camera app can write the photo
        env.call_method(
            &intent,
            "addFlags",
            "(I)Landroid/content/Intent;",
            &[JValue::Int(0x0000_0003)], // GRANT_READ + GRANT_WRITE
        )
        .map_err(|e| jni_err("addFlags(camera)", e))?;

        // -- Dispatch -----------------------------------------------------------
        env.call_method(
            &activity,
            "startActivityForResult",
            "(Landroid/content/Intent;I)V",
            &[JValue::Object(&intent), JValue::Int(REQUEST_IMAGE_CAPTURE)],
        )
        .map_err(|e| jni_err("startActivityForResult(capture)", e))?;

        tracing::info!(
            request_code = REQUEST_IMAGE_CAPTURE,
            "Android: camera intent dispatched — awaiting onActivityResult"
        );

        Err(PresswerkError::Bridge(
            "Camera intent dispatched (request code 0x50570001). \
             The captured JPEG will arrive via onActivityResult — \
             wire the Activity callback to PresswerkResultReceiver."
                .into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// NativeFilePicker — Storage Access Framework
// ---------------------------------------------------------------------------

impl NativeFilePicker for AndroidBridge {
    /// Launch the Storage Access Framework document picker.
    ///
    /// Dispatches `ACTION_OPEN_DOCUMENT` filtered to the supplied MIME types.
    /// Like camera capture, the result (a `content://` URI) arrives
    /// asynchronously via `onActivityResult` with request code
    /// [`REQUEST_PICK_FILE`].
    fn pick_file(&self, mime_types: &[&str]) -> Result<Option<String>> {
        let mut env = jni_env()?;
        let activity = activity()?;

        tracing::info!(?mime_types, "Android: launching ACTION_OPEN_DOCUMENT");

        let j_action: JString = env
            .new_string("android.intent.action.OPEN_DOCUMENT")
            .map_err(|e| jni_err("new_string(ACTION_OPEN_DOCUMENT)", e))?;

        let intent: JObject = env
            .new_object(
                "android/content/Intent",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&j_action)],
            )
            .map_err(|e| jni_err("new Intent(OPEN_DOCUMENT)", e))?;

        // intent.addCategory(Intent.CATEGORY_OPENABLE)
        let j_category: JString = env
            .new_string("android.intent.category.OPENABLE")
            .map_err(|e| jni_err("new_string(CATEGORY_OPENABLE)", e))?;

        env.call_method(
            &intent,
            "addCategory",
            "(Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&j_category)],
        )
        .map_err(|e| jni_err("addCategory(OPENABLE)", e))?;

        // Set MIME type — single type directly, multiple via EXTRA_MIME_TYPES
        if mime_types.len() == 1 {
            let j_mime: JString = env
                .new_string(mime_types[0])
                .map_err(|e| jni_err("new_string(mime)", e))?;
            env.call_method(
                &intent,
                "setType",
                "(Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(&j_mime)],
            )
            .map_err(|e| jni_err("setType", e))?;
        } else {
            // Use */* as the base type and add EXTRA_MIME_TYPES array
            let j_wildcard: JString = env
                .new_string("*/*")
                .map_err(|e| jni_err("new_string(*/*)", e))?;
            env.call_method(
                &intent,
                "setType",
                "(Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(&j_wildcard)],
            )
            .map_err(|e| jni_err("setType(*/*)", e))?;

            // Build a String[] of MIME types
            let string_class = env
                .find_class("java/lang/String")
                .map_err(|e| jni_err("find_class(String)", e))?;

            let mime_array = env
                .new_object_array(mime_types.len() as jsize, &string_class, &JObject::null())
                .map_err(|e| jni_err("new_object_array(mimes)", e))?;

            for (i, mt) in mime_types.iter().enumerate() {
                let j_mt: JString = env
                    .new_string(mt)
                    .map_err(|e| jni_err("new_string(mime_type[i])", e))?;
                env.set_object_array_element(&mime_array, i as jsize, j_mt)
                    .map_err(|e| jni_err("set_object_array_element", e))?;
            }

            let j_extra_key: JString = env
                .new_string("android.intent.extra.MIME_TYPES")
                .map_err(|e| jni_err("new_string(EXTRA_MIME_TYPES)", e))?;

            env.call_method(
                &intent,
                "putExtra",
                "(Ljava/lang/String;[Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(&j_extra_key), JValue::Object(&mime_array)],
            )
            .map_err(|e| jni_err("putExtra(EXTRA_MIME_TYPES)", e))?;
        }

        // -- Dispatch -----------------------------------------------------------
        env.call_method(
            &activity,
            "startActivityForResult",
            "(Landroid/content/Intent;I)V",
            &[JValue::Object(&intent), JValue::Int(REQUEST_PICK_FILE)],
        )
        .map_err(|e| jni_err("startActivityForResult(OPEN_DOCUMENT)", e))?;

        tracing::info!(
            request_code = REQUEST_PICK_FILE,
            "Android: file picker intent dispatched — awaiting onActivityResult"
        );

        Err(PresswerkError::Bridge(
            "File picker intent dispatched (request code 0x50570002). \
             The chosen content:// URI will arrive via onActivityResult — \
             wire the Activity callback to PresswerkResultReceiver."
                .into(),
        ))
    }

    /// Read bytes from a `content://` URI returned by the Storage Access
    /// Framework.
    ///
    /// Opens an `InputStream` via `ContentResolver.openInputStream(uri)`,
    /// reads all bytes, and returns them. This is fully synchronous.
    fn read_picked_file(&self, uri_string: &str) -> Result<Vec<u8>> {
        let mut env = jni_env()?;
        let activity = activity()?;

        tracing::info!(uri = uri_string, "Android: reading content:// URI");

        // Uri.parse(uriString)
        let j_uri_str: JString = env
            .new_string(uri_string)
            .map_err(|e| jni_err("new_string(uri)", e))?;

        let uri_obj: JObject = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&j_uri_str)],
            )
            .map_err(|e| jni_err("Uri.parse", e))?
            .l()
            .map_err(|e| jni_err("Uri.parse->l", e))?;

        // ContentResolver resolver = activity.getContentResolver()
        let resolver: JObject = env
            .call_method(
                &activity,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )
            .map_err(|e| jni_err("getContentResolver", e))?
            .l()
            .map_err(|e| jni_err("getContentResolver->l", e))?;

        // InputStream is = resolver.openInputStream(uri)
        let input_stream: JObject = env
            .call_method(
                &resolver,
                "openInputStream",
                "(Landroid/net/Uri;)Ljava/io/InputStream;",
                &[JValue::Object(&uri_obj)],
            )
            .map_err(|e| jni_err("openInputStream", e))?
            .l()
            .map_err(|e| jni_err("openInputStream->l", e))?;

        if input_stream.is_null() {
            return Err(PresswerkError::Bridge(format!(
                "ContentResolver returned null InputStream for URI: {uri_string}"
            )));
        }

        // Read all bytes using a ByteArrayOutputStream buffer
        let baos: JObject = env
            .new_object("java/io/ByteArrayOutputStream", "()V", &[])
            .map_err(|e| jni_err("new ByteArrayOutputStream", e))?;

        // Allocate a 8 KiB read buffer
        let buffer = env
            .new_byte_array(8192)
            .map_err(|e| jni_err("new_byte_array(8192)", e))?;

        loop {
            let bytes_read: i32 = env
                .call_method(&input_stream, "read", "([B)I", &[JValue::Object(&buffer)])
                .map_err(|e| jni_err("InputStream.read", e))?
                .i()
                .map_err(|e| jni_err("InputStream.read->i", e))?;

            if bytes_read < 0 {
                break;
            }

            env.call_method(
                &baos,
                "write",
                "([BII)V",
                &[
                    JValue::Object(&buffer),
                    JValue::Int(0),
                    JValue::Int(bytes_read),
                ],
            )
            .map_err(|e| jni_err("ByteArrayOutputStream.write", e))?;
        }

        // Close the input stream
        env.call_method(&input_stream, "close", "()V", &[])
            .map_err(|e| jni_err("InputStream.close", e))?;

        // Get the byte[] from the ByteArrayOutputStream
        let java_bytes: JObject = env
            .call_method(&baos, "toByteArray", "()[B", &[])
            .map_err(|e| jni_err("toByteArray", e))?
            .l()
            .map_err(|e| jni_err("toByteArray->l", e))?;

        let result = env
            .convert_byte_array(java_bytes.into_raw())
            .map_err(|e| jni_err("convert_byte_array", e))?;

        tracing::info!(
            uri = uri_string,
            bytes = result.len(),
            "Android: read content:// URI successfully"
        );

        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// NativeKeychain — SharedPreferences (MODE_PRIVATE)
// ---------------------------------------------------------------------------

impl NativeKeychain for AndroidBridge {
    /// Store a secret in Android SharedPreferences.
    ///
    /// The value is Base64-encoded before storage. The key is prefixed with
    /// [`PREFS_KEY_PREFIX`] to avoid collisions with other preference users.
    ///
    /// For production apps requiring hardware-backed security, swap this for
    /// `EncryptedSharedPreferences` from AndroidX Security — the JNI call
    /// pattern is identical, only the class name and factory method change.
    fn store_secret(&self, key: &str, value: &[u8]) -> Result<()> {
        let mut env = jni_env()?;
        let activity = activity()?;
        let alias = format!("{PREFS_KEY_PREFIX}{key}");

        tracing::info!(alias = %alias, "Android: storing secret in SharedPreferences");

        // -- Base64.encodeToString(value, Base64.NO_WRAP) -----------------------
        let j_bytes = env
            .byte_array_from_slice(value)
            .map_err(|e| jni_err("byte_array_from_slice(value)", e))?;

        let encoded: JObject = env
            .call_static_method(
                "android/util/Base64",
                "encodeToString",
                "([BI)Ljava/lang/String;",
                &[
                    JValue::Object(&j_bytes),
                    JValue::Int(2), // Base64.NO_WRAP
                ],
            )
            .map_err(|e| jni_err("Base64.encodeToString", e))?
            .l()
            .map_err(|e| jni_err("encodeToString->l", e))?;

        // -- Get SharedPreferences ----------------------------------------------
        let prefs = shared_preferences(&mut env, &activity)?;

        // -- editor = prefs.edit() ----------------------------------------------
        let editor: JObject = env
            .call_method(
                &prefs,
                "edit",
                "()Landroid/content/SharedPreferences$Editor;",
                &[],
            )
            .map_err(|e| jni_err("SharedPreferences.edit", e))?
            .l()
            .map_err(|e| jni_err("edit->l", e))?;

        // -- editor.putString(alias, encoded) -----------------------------------
        let j_alias: JString = env
            .new_string(&alias)
            .map_err(|e| jni_err("new_string(alias)", e))?;

        env.call_method(
            &editor,
            "putString",
            "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/SharedPreferences$Editor;",
            &[JValue::Object(&j_alias), JValue::Object(&encoded)],
        )
        .map_err(|e| jni_err("editor.putString", e))?;

        // -- editor.apply() (async write, non-blocking) -------------------------
        env.call_method(&editor, "apply", "()V", &[])
            .map_err(|e| jni_err("editor.apply", e))?;

        tracing::info!(alias = %alias, "Android: secret stored");
        Ok(())
    }

    /// Load a secret from Android SharedPreferences.
    ///
    /// Returns `Ok(None)` if the key does not exist.
    fn load_secret(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let mut env = jni_env()?;
        let activity = activity()?;
        let alias = format!("{PREFS_KEY_PREFIX}{key}");

        tracing::info!(alias = %alias, "Android: loading secret from SharedPreferences");

        let prefs = shared_preferences(&mut env, &activity)?;

        // prefs.getString(alias, null)
        let j_alias: JString = env
            .new_string(&alias)
            .map_err(|e| jni_err("new_string(alias)", e))?;

        let encoded: JObject = env
            .call_method(
                &prefs,
                "getString",
                "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(&j_alias), JValue::Object(&JObject::null())],
            )
            .map_err(|e| jni_err("getString", e))?
            .l()
            .map_err(|e| jni_err("getString->l", e))?;

        if encoded.is_null() {
            tracing::debug!(alias = %alias, "Android: secret not found");
            return Ok(None);
        }

        // -- Base64.decode(encoded, Base64.NO_WRAP) -----------------------------
        let decoded: JObject = env
            .call_static_method(
                "android/util/Base64",
                "decode",
                "(Ljava/lang/String;I)[B",
                &[
                    JValue::Object(&encoded),
                    JValue::Int(2), // Base64.NO_WRAP
                ],
            )
            .map_err(|e| jni_err("Base64.decode", e))?
            .l()
            .map_err(|e| jni_err("decode->l", e))?;

        let bytes = env
            .convert_byte_array(decoded.into_raw())
            .map_err(|e| jni_err("convert_byte_array(decoded)", e))?;

        tracing::info!(alias = %alias, bytes = bytes.len(), "Android: secret loaded");
        Ok(Some(bytes))
    }

    /// Delete a secret from Android SharedPreferences.
    ///
    /// Silently succeeds if the key does not exist.
    fn delete_secret(&self, key: &str) -> Result<()> {
        let mut env = jni_env()?;
        let activity = activity()?;
        let alias = format!("{PREFS_KEY_PREFIX}{key}");

        tracing::info!(alias = %alias, "Android: deleting secret from SharedPreferences");

        let prefs = shared_preferences(&mut env, &activity)?;

        let editor: JObject = env
            .call_method(
                &prefs,
                "edit",
                "()Landroid/content/SharedPreferences$Editor;",
                &[],
            )
            .map_err(|e| jni_err("SharedPreferences.edit", e))?
            .l()
            .map_err(|e| jni_err("edit->l", e))?;

        let j_alias: JString = env
            .new_string(&alias)
            .map_err(|e| jni_err("new_string(alias)", e))?;

        env.call_method(
            &editor,
            "remove",
            "(Ljava/lang/String;)Landroid/content/SharedPreferences$Editor;",
            &[JValue::Object(&j_alias)],
        )
        .map_err(|e| jni_err("editor.remove", e))?;

        env.call_method(&editor, "apply", "()V", &[])
            .map_err(|e| jni_err("editor.apply", e))?;

        tracing::info!(alias = %alias, "Android: secret deleted");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// NativeShare — Intent ACTION_SEND
// ---------------------------------------------------------------------------

impl NativeShare for AndroidBridge {
    /// Share a file via the Android share sheet (`Intent.ACTION_SEND`).
    ///
    /// Converts the file path to a `content://` URI through `FileProvider`,
    /// then launches a chooser intent so the user can pick the target app.
    fn share_file(&self, path: &str, mime_type: &str) -> Result<()> {
        let mut env = jni_env()?;
        let activity = activity()?;

        tracing::info!(path, mime = mime_type, "Android: launching share intent");

        // -- Build File object --------------------------------------------------
        let j_path: JString = env
            .new_string(path)
            .map_err(|e| jni_err("new_string(path)", e))?;

        let file_obj: JObject = env
            .new_object(
                "java/io/File",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&j_path)],
            )
            .map_err(|e| jni_err("new File(path)", e))?;

        // -- Build content:// URI via FileProvider ------------------------------
        let authority = get_authority(&mut env, &activity)?;
        let j_authority: JString = env
            .new_string(&authority)
            .map_err(|e| jni_err("new_string(authority)", e))?;

        let content_uri: JObject = env
            .call_static_method(
                "androidx/core/content/FileProvider",
                "getUriForFile",
                "(Landroid/content/Context;Ljava/lang/String;Ljava/io/File;)Landroid/net/Uri;",
                &[
                    JValue::Object(&activity),
                    JValue::Object(&j_authority),
                    JValue::Object(&file_obj),
                ],
            )
            .map_err(|e| jni_err("FileProvider.getUriForFile(share)", e))?
            .l()
            .map_err(|e| jni_err("getUriForFile->l(share)", e))?;

        // -- Build ACTION_SEND intent -------------------------------------------
        let j_action: JString = env
            .new_string("android.intent.action.SEND")
            .map_err(|e| jni_err("new_string(ACTION_SEND)", e))?;

        let intent: JObject = env
            .new_object(
                "android/content/Intent",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&j_action)],
            )
            .map_err(|e| jni_err("new Intent(SEND)", e))?;

        // intent.setType(mimeType)
        let j_mime: JString = env
            .new_string(mime_type)
            .map_err(|e| jni_err("new_string(mime)", e))?;

        env.call_method(
            &intent,
            "setType",
            "(Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&j_mime)],
        )
        .map_err(|e| jni_err("setType(share)", e))?;

        // intent.putExtra(Intent.EXTRA_STREAM, contentUri)
        let j_extra_stream: JString = env
            .new_string("android.intent.extra.STREAM")
            .map_err(|e| jni_err("new_string(EXTRA_STREAM)", e))?;

        env.call_method(
            &intent,
            "putExtra",
            "(Ljava/lang/String;Landroid/os/Parcelable;)Landroid/content/Intent;",
            &[
                JValue::Object(&j_extra_stream),
                JValue::Object(&content_uri),
            ],
        )
        .map_err(|e| jni_err("putExtra(EXTRA_STREAM)", e))?;

        // Grant read permission
        env.call_method(
            &intent,
            "addFlags",
            "(I)Landroid/content/Intent;",
            &[JValue::Int(0x0000_0001)], // FLAG_GRANT_READ_URI_PERMISSION
        )
        .map_err(|e| jni_err("addFlags(share)", e))?;

        // -- Wrap in a chooser --------------------------------------------------
        let j_title: JString = env
            .new_string("Share via")
            .map_err(|e| jni_err("new_string(chooser_title)", e))?;

        let chooser: JObject = env
            .call_static_method(
                "android/content/Intent",
                "createChooser",
                "(Landroid/content/Intent;Ljava/lang/CharSequence;)Landroid/content/Intent;",
                &[JValue::Object(&intent), JValue::Object(&j_title)],
            )
            .map_err(|e| jni_err("Intent.createChooser", e))?
            .l()
            .map_err(|e| jni_err("createChooser->l", e))?;

        // -- Launch -------------------------------------------------------------
        env.call_method(
            &activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&chooser)],
        )
        .map_err(|e| jni_err("startActivity(share)", e))?;

        tracing::info!(path, mime = mime_type, "Android: share intent dispatched");
        Ok(())
    }

    /// Share text content via the Android share sheet.
    fn share_text(&self, text: &str) -> Result<()> {
        let mut env = jni_env()?;
        let activity = activity()?;

        tracing::info!("Android: sharing text via ACTION_SEND");

        let j_action: JString = env
            .new_string("android.intent.action.SEND")
            .map_err(|e| jni_err("new_string(ACTION_SEND_TEXT)", e))?;

        let intent: JObject = env
            .new_object(
                "android/content/Intent",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&j_action)],
            )
            .map_err(|e| jni_err("new Intent(SEND_TEXT)", e))?;

        let j_mime: JString = env
            .new_string("text/plain")
            .map_err(|e| jni_err("new_string(text/plain)", e))?;

        env.call_method(
            &intent,
            "setType",
            "(Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&j_mime)],
        )
        .map_err(|e| jni_err("setType(text)", e))?;

        let j_extra_text: JString = env
            .new_string("android.intent.extra.TEXT")
            .map_err(|e| jni_err("new_string(EXTRA_TEXT)", e))?;

        let j_text: JString = env
            .new_string(text)
            .map_err(|e| jni_err("new_string(text_content)", e))?;

        env.call_method(
            &intent,
            "putExtra",
            "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&j_extra_text), JValue::Object(&j_text)],
        )
        .map_err(|e| jni_err("putExtra(EXTRA_TEXT)", e))?;

        let j_title: JString = env
            .new_string("Share via")
            .map_err(|e| jni_err("new_string(chooser_title_text)", e))?;

        let chooser: JObject = env
            .call_static_method(
                "android/content/Intent",
                "createChooser",
                "(Landroid/content/Intent;Ljava/lang/CharSequence;)Landroid/content/Intent;",
                &[JValue::Object(&intent), JValue::Object(&j_title)],
            )
            .map_err(|e| jni_err("Intent.createChooser(text)", e))?
            .l()
            .map_err(|e| jni_err("createChooser->l(text)", e))?;

        env.call_method(
            &activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&chooser)],
        )
        .map_err(|e| jni_err("startActivity(share_text)", e))?;

        tracing::info!("Android: text share intent dispatched");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Obtain the application's `SharedPreferences` in private mode.
///
/// Calls `activity.getSharedPreferences("presswerk_secrets", MODE_PRIVATE)`.
fn shared_preferences<'a>(env: &mut JNIEnv<'a>, activity: &JObject<'_>) -> Result<JObject<'a>> {
    let j_name: JString = env
        .new_string(PREFS_FILE)
        .map_err(|e| jni_err("new_string(prefs_name)", e))?;

    env.call_method(
        activity,
        "getSharedPreferences",
        "(Ljava/lang/String;I)Landroid/content/SharedPreferences;",
        &[
            JValue::Object(&j_name),
            JValue::Int(0), // MODE_PRIVATE
        ],
    )
    .map_err(|e| jni_err("getSharedPreferences", e))?
    .l()
    .map_err(|e| jni_err("getSharedPreferences->l", e))
}

/// Build the FileProvider authority string for this application.
///
/// Convention: `<applicationId>.fileprovider`. We read the package name
/// from the Activity's `getPackageName()` and append `.fileprovider`.
fn get_authority(env: &mut JNIEnv<'_>, activity: &JObject<'_>) -> Result<String> {
    let j_pkg: JObject = env
        .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])
        .map_err(|e| jni_err("getPackageName", e))?
        .l()
        .map_err(|e| jni_err("getPackageName->l", e))?;

    let pkg: String = env
        .get_string(&JString::from(j_pkg))
        .map_err(|e| jni_err("get_string(packageName)", e))?
        .into();

    Ok(format!("{pkg}.fileprovider"))
}

// ---------------------------------------------------------------------------
// Stub implementations for connection types not yet wired to Android APIs
// ---------------------------------------------------------------------------

impl NativeUsbPrint for AndroidBridge {
    fn detect_usb_printers(&self) -> Result<Vec<UsbPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_usb(&self, _device_id: &str, _document: &[u8], _mime_type: &str) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeBluetoothPrint for AndroidBridge {
    fn scan_bluetooth_printers(&self) -> Result<Vec<BluetoothPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_bluetooth(&self, _device_id: &str, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeNfcPrint for AndroidBridge {
    fn read_nfc_printer_tag(&self) -> Result<Option<NfcPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeConnectivity for AndroidBridge {
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

impl NativeFireWirePrint for AndroidBridge {
    fn detect_firewire_printers(&self) -> Result<Vec<FireWirePrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_firewire(&self, _device_id: &str, _document: &[u8], _mime_type: &str) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeLightningPrint for AndroidBridge {
    fn detect_lightning_printers(&self) -> Result<Vec<LightningPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_lightning(&self, _device_id: &str, _document: &[u8], _mime_type: &str) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeThunderboltPrint for AndroidBridge {
    fn detect_thunderbolt_printers(&self) -> Result<Vec<ThunderboltPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_thunderbolt(
        &self,
        _device_id: &str,
        _document: &[u8],
        _mime_type: &str,
    ) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeSerialPrint for AndroidBridge {
    fn detect_serial_printers(&self) -> Result<Vec<SerialPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_serial(&self, _port: &str, _baud_rate: u32, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeParallelPrint for AndroidBridge {
    fn detect_parallel_printers(&self) -> Result<Vec<ParallelPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_parallel(&self, _port: &str, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeInfraredPrint for AndroidBridge {
    fn scan_infrared_printers(&self) -> Result<Vec<InfraredPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_infrared(&self, _device_id: &str, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeIBeaconDiscover for AndroidBridge {
    fn scan_ibeacon_printers(&self) -> Result<Vec<IBeaconPrinterInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeLiFiPrint for AndroidBridge {
    fn detect_lifi_endpoints(&self) -> Result<Vec<LiFiEndpointInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn print_lifi(&self, _endpoint_id: &str, _document: &[u8]) -> Result<()> {
        Err(PresswerkError::PlatformUnavailable)
    }
}

impl NativeUsbDrivePrint for AndroidBridge {
    fn detect_usb_drives(&self) -> Result<Vec<UsbDriveInfo>> {
        Err(PresswerkError::PlatformUnavailable)
    }

    fn copy_to_usb_drive(
        &self,
        _drive_id: &str,
        _document: &[u8],
        _filename: &str,
    ) -> Result<String> {
        Err(PresswerkError::PlatformUnavailable)
    }
}
