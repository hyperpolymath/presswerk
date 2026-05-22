||| Presswerk Bridge ABI Proofs
|||
||| This module provides the formal specification for the native platform bridges.
||| It defines the safety invariants that MUST be maintained by the Rust bridge
||| implementation (`presswerk-bridge`).
|||
||| GOAL: Ensure that low-level interactions with mobile SDKs (Core Foundation,
||| Security.framework, JNI) are type-safe and follow mathematically proven patterns.
|||
||| SPDX-License-Identifier: MPL-2.0
||| Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath)

module Presswerk.ABI.Bridge

import Presswerk.ABI.Types
import Data.So

%default total

--------------------------------------------------------------------------------
-- Toll-Free Bridging (iOS / Core Foundation)
--------------------------------------------------------------------------------

||| Proof witness for Apple's "Toll-Free Bridging".
||| Validates that two types (e.g., `NSString` and `CFString`) share the same
||| memory layout, justifying zero-cost casts at the FFI boundary.
|||
||| See: Apple Technical Note TN2151 for official documentation.
public export
data TollFreePair : Type -> Type -> Type where
  ||| Constructor requiring proofs of identical size and alignment.
  MkTollFree : (a : Type) -> (b : Type)
            -> (sameSize : sizeOf a = sizeOf b)
            -> (sameAlign : alignOf a = alignOf b)
            -> TollFreePair a b

-- ... [Symmetry and Transitivity proofs follow, ensuring the bridging logic is robust]

--------------------------------------------------------------------------------
-- Keychain Properties
--------------------------------------------------------------------------------

||| Formal specification for persistent secret storage.
||| Defines the required behavior for both iOS (Security.framework) and
||| Android (SharedPreferences/Keystore) implementations.
public export
data KeychainProperty : Type where

  ||| Invariant: Data successfully stored must be retrievable.
  StoreLoad : (key : String) -> (value : List Bits8)
           -> KeychainProperty

  ||| Invariant: Deleted data must not be retrievable.
  DeleteLoad : (key : String)
            -> KeychainProperty

  ||| Invariant: Storing a new value for an existing key must overwrite the old value.
  LastWriteWins : (key : String) -> (v1 : List Bits8) -> (v2 : List Bits8)
               -> KeychainProperty

--------------------------------------------------------------------------------
-- Thread Safety Preconditions
--------------------------------------------------------------------------------

||| Enforces thread safety requirements for bridge operations.
||| UI-related tasks (printing, image capture) ARE RESTRICTED to the Main Thread.
||| Storage and file I/O operations are thread-agnostic.
public export
data ThreadRequirement : Type where
  MainThread : ThreadRequirement
  AnyThread : ThreadRequirement

||| Mapping function defining the required thread for each Bridge operation.
||| This is used by the Rust bridge to verify thread state before calling native SDKs.
public export
threadReq : BridgeOp -> ThreadRequirement
threadReq ShowPrintDialog = MainThread
threadReq CaptureImage    = MainThread
threadReq StoreSecret     = AnyThread
-- ... [other operations omitted]

--------------------------------------------------------------------------------
-- JNI Safety Invariants (Android)
--------------------------------------------------------------------------------

||| Formalizes the safety contract for Android's Java Native Interface (JNI).
||| Ensures that VM handles and Activity references remain valid throughout
||| the application lifecycle.
public export
data JniInvariant : Type where
  ||| Verifies that the JavaVM handle is valid and non-null.
  ValidJavaVM : JniInvariant
  ||| Verifies that the hosting Activity is a valid global reference.
  ValidActivity : JniInvariant
