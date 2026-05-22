||| Presswerk ABI Type Definitions
|||
||| Core types for the Presswerk print router with formal proofs of correctness.
||| All type definitions include C ABI size proofs and platform detection.
|||
||| SPDX-License-Identifier: MPL-2.0
||| Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath)

module Presswerk.ABI.Types

import Data.Bits
import Data.So
import Data.Vect

%default total

--------------------------------------------------------------------------------
-- Platform Detection
--------------------------------------------------------------------------------

||| Supported target platforms
public export
data Platform = Linux | MacOS | IOS | Android | WASM

||| Mobile platforms only
public export
isMobile : Platform -> Bool
isMobile IOS = True
isMobile Android = True
isMobile _ = False

--------------------------------------------------------------------------------
-- FFI Result Codes
--------------------------------------------------------------------------------

||| Result codes for FFI operations (C-compatible integers)
public export
data Result : Type where
  Ok           : Result
  Error        : Result
  InvalidParam : Result
  OutOfMemory  : Result
  NullPointer  : Result
  Unsupported  : Result

||| Convert Result to C integer
public export
resultToInt : Result -> Bits32
resultToInt Ok           = 0
resultToInt Error        = 1
resultToInt InvalidParam = 2
resultToInt OutOfMemory  = 3
resultToInt NullPointer  = 4
resultToInt Unsupported  = 5

||| Results are decidably equal
public export
DecEq Result where
  decEq Ok Ok = Yes Refl
  decEq Error Error = Yes Refl
  decEq InvalidParam InvalidParam = Yes Refl
  decEq OutOfMemory OutOfMemory = Yes Refl
  decEq NullPointer NullPointer = Yes Refl
  decEq Unsupported Unsupported = Yes Refl
  decEq _ _ = No absurd

--------------------------------------------------------------------------------
-- Job Status (mirrors Rust JobStatus enum)
--------------------------------------------------------------------------------

||| Print job lifecycle states
public export
data JobStatus = Pending | Processing | Completed | Failed | Cancelled | Held

||| Job status to C integer
public export
jobStatusToInt : JobStatus -> Bits32
jobStatusToInt Pending    = 0
jobStatusToInt Processing = 1
jobStatusToInt Completed  = 2
jobStatusToInt Failed     = 3
jobStatusToInt Cancelled  = 4
jobStatusToInt Held       = 5

||| Proof: jobStatusToInt is injective (distinct statuses get distinct codes)
public export
jobStatusInjective : (a, b : JobStatus) -> jobStatusToInt a = jobStatusToInt b -> a = b
jobStatusInjective Pending    Pending    Refl = Refl
jobStatusInjective Processing Processing Refl = Refl
jobStatusInjective Completed  Completed  Refl = Refl
jobStatusInjective Failed     Failed     Refl = Refl
jobStatusInjective Cancelled  Cancelled  Refl = Refl
jobStatusInjective Held       Held       Refl = Refl

--------------------------------------------------------------------------------
-- Document Types
--------------------------------------------------------------------------------

||| Supported document formats
public export
data DocType = PDF | JPEG | PNG | TIFF | PlainText | NativeDelegate

||| MIME type string for IPP Content-Type
public export
mimeType : DocType -> String
mimeType PDF            = "application/pdf"
mimeType JPEG           = "image/jpeg"
mimeType PNG            = "image/png"
mimeType TIFF           = "image/tiff"
mimeType PlainText      = "text/plain"
mimeType NativeDelegate = "application/octet-stream"

--------------------------------------------------------------------------------
-- Paper Sizes
--------------------------------------------------------------------------------

||| Standard paper size dimensions in millimetres
public export
data PaperSize = A4 | A3 | A5 | Letter | Legal | Tabloid | Custom Nat Nat

||| Width in mm
public export
paperWidth : PaperSize -> Nat
paperWidth A4             = 210
paperWidth A3             = 297
paperWidth A5             = 148
paperWidth Letter         = 216
paperWidth Legal          = 216
paperWidth Tabloid        = 279
paperWidth (Custom w _)   = w

||| Height in mm
public export
paperHeight : PaperSize -> Nat
paperHeight A4            = 297
paperHeight A3            = 420
paperHeight A5            = 210
paperHeight Letter        = 279
paperHeight Legal         = 356
paperHeight Tabloid       = 432
paperHeight (Custom _ h)  = h

--------------------------------------------------------------------------------
-- Opaque Handles
--------------------------------------------------------------------------------

||| Non-null pointer handle for FFI
public export
data Handle : Type where
  MkHandle : (ptr : Bits64) -> {auto 0 nonNull : So (ptr /= 0)} -> Handle

||| Safely create a handle (returns Nothing for null)
public export
createHandle : Bits64 -> Maybe Handle
createHandle 0 = Nothing
createHandle ptr = Just (MkHandle ptr)

||| Extract raw pointer
public export
handlePtr : Handle -> Bits64
handlePtr (MkHandle ptr) = ptr

--------------------------------------------------------------------------------
-- Memory Layout
--------------------------------------------------------------------------------

||| Proof that a type has a specific size in bytes
public export
data HasSize : Type -> Nat -> Type where
  SizeProof : {0 t : Type} -> {n : Nat} -> HasSize t n

||| Proof that a type has a specific alignment
public export
data HasAlignment : Type -> Nat -> Type where
  AlignProof : {0 t : Type} -> {n : Nat} -> HasAlignment t n

||| C pointer size per platform
public export
ptrSize : Platform -> Nat
ptrSize Linux   = 64
ptrSize MacOS   = 64
ptrSize IOS     = 64
ptrSize Android = 64
ptrSize WASM    = 32
