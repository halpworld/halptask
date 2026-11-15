# TEST-ONLY signing key. NEVER TRUST IT.

`NEVER-TRUST-test-only.key` is the private signing key of the example pack
`hub/examples/packs/acme-sre`. It is committed on purpose so that the
example and its test can be rebuilt, which means **anyone** can sign a pack
with it.

- Do not add this key (`NEVER-TRUST-test-only.pub`) to a real Hub's trusted
  keys except on a throwaway Hub to try the example.
- Do not use it for anything you publish. Make your own key with
  `halpd pack keygen --out my.key`.
- It is not, and must never be, a first-party key.

`NEVER-TRUST-test-only.pub` holds the public key (base64) for Settings >
Packs > Trusted keys.
