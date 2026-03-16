// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

||| Presswerk — Native platform bridge abstractions.
|||
||| This module defines the core traits and platform dispatch logic for the
||| native SDK bridge. It allows the high-level Rust code to interact with
||| iOS (Core Foundation) and Android (ART/JNI) APIs through a unified interface.
|||
||| SECURITY: Implementations must adhere to the proofs in `src/abi/Bridge.idr`.

pub mod traits;

#[cfg(target_os = "ios")]
pub mod ios;

#[cfg(target_os = "android")]
pub mod android;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod stub;

/// Retrieves the singleton bridge implementation for the target operating system.
/// 
/// RETURNS: A boxed trait object (`dyn PlatformBridge`) that abstracts away
/// the underlying native SDK details.
pub fn platform_bridge() -> Box<dyn traits::PlatformBridge> {
    #[cfg(target_os = "ios")]
    {
        // iOS: Uses `objc2` for type-safe message passing to Objective-C.
        Box::new(ios::IosBridge::new())
    }
    #[cfg(target_os = "android")]
    {
        // Android: Uses `jni-rs` to invoke methods on the JVM/ART.
        Box::new(android::AndroidBridge::new())
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        // DESKTOP/CI: Uses a mock implementation to allow non-native builds.
        Box::new(stub::StubBridge)
    }
}
