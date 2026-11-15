;; acme.platform-sre / burn: counts the open incidents in its input.
;;
;; Tier B ABI (hub/crates/halp-ext): exports `memory`, `halp_alloc(len) ->
;; ptr` and `halp_transform(ptr, len) -> (out_ptr << 32 | out_len)`. It has
;; no imports at all: no clock, no randomness, no files, no network.
;;
;; The input is the JSON envelope {"inputs":{"open":<tasks.list>},"config":..,
;; "now":..,"today":..}. The manifest's `open` input lists the unfinished
;; items of type acme.incident, so each one carries the exact bytes
;;   "type_id":"acme.incident"
;; and the number of those is the open-incident count. (A user-typed string
;; holding that text is JSON-escaped, so it cannot match.)
;;
;; Output: {"value":<n>,"label":"Open incidents","hint":"..."}, the
;; `metric` view of a `computed` widget.
;;
;; Rebuild wasm/burn.wasm (and the manifest's file index) from this file with
;;   HALP_REGEN_EXAMPLE=1 cargo test -p halp-ext --test example_pack
(module
  (memory (export "memory") 2)

  ;; 0..25: the pattern to count.
  (data (i32.const 0) "\22type_id\22:\22acme.incident\22")
  ;; 64..73: the output prefix.
  (data (i32.const 64) "{\22value\22:")
  ;; 96..: the output suffix.
  (data (i32.const 96) ",\22label\22:\22Open incidents\22,\22hint\22:\22unfinished items of type acme.incident\22}")

  (global $next (mut i32) (i32.const 1024))

  (func $alloc (export "halp_alloc") (param $len i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $next))
    (if (i32.gt_u (i32.add (local.get $p) (local.get $len))
                  (i32.mul (memory.size) (i32.const 65536)))
      (then (drop (memory.grow
        (i32.add (i32.div_u (local.get $len) (i32.const 65536)) (i32.const 1))))))
    (global.set $next (i32.add (local.get $p) (local.get $len)))
    (local.get $p))

  (func (export "halp_transform") (param $ptr i32) (param $len i32) (result i64)
    (local $i i32) (local $j i32) (local $count i32) (local $hit i32)
    (local $out i32) (local $w i32) (local $n i32) (local $d i32) (local $tmp i32)

    ;; Count non-overlapping occurrences of the 25-byte pattern.
    (block $done
      (loop $scan
        (br_if $done (i32.gt_u (i32.add (local.get $i) (i32.const 25)) (local.get $len)))
        (local.set $hit (i32.const 1))
        (local.set $j (i32.const 0))
        (block $cmp_end
          (loop $cmp
            (br_if $cmp_end (i32.ge_u (local.get $j) (i32.const 25)))
            (if (i32.ne
                  (i32.load8_u (i32.add (local.get $ptr) (i32.add (local.get $i) (local.get $j))))
                  (i32.load8_u (local.get $j)))
              (then (local.set $hit (i32.const 0)) (br $cmp_end)))
            (local.set $j (i32.add (local.get $j) (i32.const 1)))
            (br $cmp)))
        (if (local.get $hit)
          (then
            (local.set $count (i32.add (local.get $count) (i32.const 1)))
            (local.set $i (i32.add (local.get $i) (i32.const 25))))
          (else (local.set $i (i32.add (local.get $i) (i32.const 1)))))
        (br $scan)))

    ;; Output buffer: prefix (9) + up to 10 digits + suffix (74).
    (local.set $out (call $alloc (i32.const 128)))
    (local.set $w (local.get $out))
    (memory.copy (local.get $w) (i32.const 64) (i32.const 9))
    (local.set $w (i32.add (local.get $w) (i32.const 9)))

    ;; The count in decimal: digits backwards into 48..58, then forwards.
    (local.set $n (local.get $count))
    (local.set $tmp (i32.const 58))
    (if (i32.eqz (local.get $n))
      (then
        (local.set $tmp (i32.sub (local.get $tmp) (i32.const 1)))
        (i32.store8 (local.get $tmp) (i32.const 48))))
    (block $digits_end
      (loop $digits
        (br_if $digits_end (i32.eqz (local.get $n)))
        (local.set $d (i32.rem_u (local.get $n) (i32.const 10)))
        (local.set $tmp (i32.sub (local.get $tmp) (i32.const 1)))
        (i32.store8 (local.get $tmp) (i32.add (local.get $d) (i32.const 48)))
        (local.set $n (i32.div_u (local.get $n) (i32.const 10)))
        (br $digits)))
    (memory.copy (local.get $w) (local.get $tmp) (i32.sub (i32.const 58) (local.get $tmp)))
    (local.set $w (i32.add (local.get $w) (i32.sub (i32.const 58) (local.get $tmp))))

    (memory.copy (local.get $w) (i32.const 96) (i32.const 74))
    (local.set $w (i32.add (local.get $w) (i32.const 74)))

    (i64.or
      (i64.shl (i64.extend_i32_u (local.get $out)) (i64.const 32))
      (i64.extend_i32_u (i32.sub (local.get $w) (local.get $out))))))
