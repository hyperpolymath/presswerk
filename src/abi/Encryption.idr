||| Presswerk ABI Encryption Invariants
|||
||| This module defines the formal specification for the encryption layer. 
||| It uses "Axiomatic Proofs" to document the mathematical properties that 
||| the native Rust/Zig implementation MUST satisfy.
|||
||| NOTE: Actual cryptographic execution happens in the Rust `presswerk-security` 
||| crate using the `age` and `ring` libraries. This Idris module serves as the 
||| high-level correctness contract.
|||
||| SPDX-License-Identifier: MPL-2.0
||| Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath)

module Presswerk.ABI.Encryption

import Data.Vect

%default total

--------------------------------------------------------------------------------
-- Abstract Crypto Types
--------------------------------------------------------------------------------

||| Abstract representation of a cryptographic key.
public export
data Key : Type where
  EncKey : Key -- Public or Symmetric key for encryption
  DecKey : Key -- Private or Symmetric key for decryption

||| An identity's key material.
public export
record KeyPair where
  constructor MkKeyPair
  encryptionKey : Key
  decryptionKey : Key

||| Binary data representation.
public export
Plaintext : Type
Plaintext = List Bits8

||| Encrypted binary data representation.
public export
Ciphertext : Type
Ciphertext = List Bits8

--------------------------------------------------------------------------------
-- Required Properties (Formal Specification)
--------------------------------------------------------------------------------

||| CORE INVARIANT: Decryption is the inverse of encryption.
||| Any implementation of the bridge must guarantee that data encrypted with 
||| a key can be recovered using the matching decryption key.
public export
0 RoundtripProperty : Type
RoundtripProperty =
  (kp : KeyPair) -> (p : Plaintext) ->
  decrypt (decryptionKey kp) (encrypt (encryptionKey kp) p) = Just p

||| SECURITY PROPERTY: Unauthorized decryption fails.
||| Attempting to decrypt with an incorrect key MUST return `Nothing`.
public export
0 WrongKeyFails : Type
WrongKeyFails =
  (kp1, kp2 : KeyPair) -> Not (kp1 = kp2) ->
  (p : Plaintext) -> NonEmpty p ->
  decrypt (decryptionKey kp2) (encrypt (encryptionKey kp1) p) = Nothing

--------------------------------------------------------------------------------
-- Hash Properties (SHA-256)
--------------------------------------------------------------------------------

||| INVARIANT: The SHA-256 hash algorithm MUST always produce a 256-bit (32-byte) digest.
public export
0 HashOutputSize : Type
HashOutputSize =
  (input : Plaintext) -> length (sha256 input) = 32
  where
    sha256 : Plaintext -> Vect 32 Bits8
    sha256 _ = replicate 32 0 -- Axiom implemented in native code
