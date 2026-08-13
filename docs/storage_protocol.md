# HalpTask Storage Protocol Specification 💾

HalpTask uses a high-performance, compact **Protobuf v1 binary protocol** (`.pb`) for data persistence, paired with **automatic legacy Markdown migration** and optional **AES-256-GCM encryption**.

---

## 1. Storage Architecture Overview

HalpTask storage files consist of either:
1. **Unencrypted Binary Payload**: A Protobuf binary stream containing the full item tree and document metadata.
2. **Encrypted Payload**: An unencrypted text header (`# HALPTASK-ENCRYPTED-v1\n`), followed by a 16-byte random salt, 12-byte GCM nonce, and AES-256-GCM ciphertext wrapping the Protobuf payload.

---

## 2. Protobuf Schema Specification (`proto/v1/storage.proto`)

HalpTask uses Schema Version `1` with the following Protobuf message structure:

```protobuf
syntax = "proto3";

package halptask.v1;

enum TaskStatus {
  TASK_STATUS_UNSPECIFIED = 0;
  TASK_STATUS_NONE = 1;
  TASK_STATUS_TODO = 2;
  TASK_STATUS_IN_PROGRESS = 3;
  TASK_STATUS_DONE = 4;
}

message ItemProto {
  string id = 1;              // Unique item identifier (e.g., "1", "42")
  string text = 2;            // Item display text / title
  bool is_task = 3;           // true if item is a task checkbox
  TaskStatus status = 4;      // TASK_STATUS_TODO, TASK_STATUS_IN_PROGRESS, TASK_STATUS_DONE
  bool folded = 5;            // Subtree collapse state
  repeated string tags = 6;   // Direct tag strings (e.g., "bug", "urgent")
  repeated ItemProto children = 7; // Recursive child items

  // Sync & Decentralized Metadata
  int64 created_at = 8;       // Unix timestamp (nanoseconds)
  int64 updated_at = 9;       // Unix timestamp (nanoseconds)
  string node_id = 10;        // Decentralized node generator ID
  bool deleted = 11;          // Tombstone deletion flag
  uint64 version = 12;        // Item mutation sequence number
  string note = 13;           // Attached Markdown note text
}

message TreeProto {
  uint32 schema_version = 1;  // Schema version (currently uint32(1))
  repeated ItemProto roots = 2; // Root-level items
  int64 last_modified = 3;    // Unix timestamp of last file write
}
```

---

## 3. Binary Wire Format & Encoding

HalpTask uses a lightweight, zero-dependency binary encoder/decoder in [`proto/v1/storage.pb.go`](file:///Users/kenth/code/halptask/proto/v1/storage.pb.go):

- **Varint Encoding**: Standard Protobuf unsigned varint encoding for field keys, booleans (`0` / `1`), enums (`TaskStatus`), integers (`created_at`, `version`), and lengths.
- **Length-Delimited Encoding**: Wire type 2 for strings (`id`, `text`, `node_id`, `note`, `tags`) and embedded child messages (`ItemProto`).

---

## 4. Encryption Wrapper Specification

When encryption is enabled (`encrypted: true` in config or `-e` flag):

```
┌─────────────────────────────────────────────────────────────┐
│ Header: # HALPTASK-ENCRYPTED-v1\n                           │
├─────────────────────────────────────────────────────────────┤
│ Salt:   16 bytes (crypto/rand salt for PBKDF2)              │
├─────────────────────────────────────────────────────────────┤
│ Nonce:  12 bytes (crypto/rand AES-GCM nonce)                │
├─────────────────────────────────────────────────────────────┤
│ Payload: AES-256-GCM Ciphertext + 16-byte Authentication Tag │
│          (Encrypts raw Protobuf binary payload)             │
└─────────────────────────────────────────────────────────────┘
```

### Encryption Algorithm:
- **Cipher**: **AES-256-GCM** (Galois/Counter Mode).
- **Key Derivation Function**: **PBKDF2-HMAC-SHA256** with **100,000 iterations**.
- **Encoding**: Binary salt & nonce concatenated with GCM ciphertext, encoded in standard Base64 string payload following the text header.

---

## 5. Automatic Legacy Migration (`MigrateIfNeeded`)

When opening legacy `.txt` or `.md` files:
1. HalpTask checks if the file begins with Protobuf binary wire format or the Encrypted Header.
2. If legacy Markdown format is detected:
   - Parses tree structure, task statuses (`[ ]`, `[~]`, `[x]`), IDs (`<!-- id: XYZ -->`), and tags (`#tagname`).
   - Creates a backup file (`<filename>.bak`).
   - Automatically migrates data to Protobuf v1 binary format and renames file path extension to `.pb`.
