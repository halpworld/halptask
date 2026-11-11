# acme.platform-sre: an example HalpTask Hub pack

A small, complete extension pack you can fork. It proves the whole path: a
role that extends `halp.developer`, a task type, a prompt, a sandboxed
WebAssembly transform behind a `computed` widget (Tier B), and a sandboxed
HTML widget (Tier C).

| Part | File | Tier |
|---|---|---|
| Manifest | `halp-pack.json` | |
| Role and task type `acme.incident` | manifest (`extends`, `task_types`) | A (data) |
| Prompt `oncall-handoff` | `prompts/oncall-handoff.md` | A |
| Transform `acme.platform-sre/burn` (open-incident count) | `wasm/burn.wat` -> `wasm/burn.wasm` | B |
| Widget `x.acme.platform-sre.oncall` | `ui/oncall/index.html`, `ui/oncall/oncall.js` | C |

The committed bundle is `../acme-sre.halppack`, signed with the **test-only**
key in `../../keys/` (see the warning there). Never trust that key: anyone
can sign with it. It exists so this example and its test are reproducible.

## Try it

1. In the Hub, open Settings > Packs and choose `acme-sre.halppack`.
2. The review shows the pack as signed by an unknown key. To install it you
   must trust the key (the "Trust this key" choice); only do that for a
   key you know. For this example the key is the test key, so do it on a
   throwaway Hub only.
3. Switch to the role "Platform SRE". Add a **Computed** widget and set its
   config to `{"transform": "acme.platform-sre/burn", "view": "metric"}`,
   and add the **On-call** widget. Create tasks of type `Incident`.

Tier C widgets (the On-call widget) run on macOS and Linux only. On
Windows the pack installs, and Tier B and the prompt work, but the widget
shows a placeholder.

## Fork it and ship your own

1. Copy this folder and pick your own id (`yourorg.something`, two to five
   dot-separated lowercase segments; `halp.*` is reserved). Change `id`,
   `title`, `publisher`, `role.id` and the widget/transform names. The
   widget kind is `x.<pack id>.<widget name>`, and the transform name in a
   `computed` config is `<pack id>/<module name>`.
2. Edit the parts you want. Rules to keep in mind:
   - The WebAssembly module may import nothing but `halp.log`; it gets only
     the read-only inputs the manifest lists (`tasks.list`, `tasks.get`,
     `types.list`, `types.get`, `day.state`, `focus.get`, `layout.get`) and
     must return JSON that fits its `output_schema` (and one of the
     `computed` views: metric, list, series, table).
   - The HTML widget may not use inline scripts, the network or
     `eval`: its CSP only allows scripts from the widget's own files. It
     may call only the tools in `calls` (`tasks_list`, `tasks_get`,
     `types_list`, `types_get`, `day_state`, `focus_get`). Set text with
     `textContent`.
3. Rebuild the module and the file index. `wasm/burn.wat` is compiled by
   the repo's test (it holds the only WAT compiler the build uses):
   `HALP_REGEN_EXAMPLE=1 cargo test -p halp-ext --test example_pack`
   rewrites `wasm/burn.wasm` and the `files` index in `halp-pack.json`. If you
   use another toolchain, produce `wasm/*.wasm` yourself and list every
   file's `sha256` and `size` in `files` (every file in the folder except
   `halp-pack.json` must be listed, and nothing else may be present).
4. Make your own signing key and sign. `halpd pack sign` checks the bundle
   as an installer would once your key is trusted, and `halpd pack verify
   --trust <file.pub>` checks it against a public key (base64, as printed by
   `keygen`). (`HALP_REGEN_EXAMPLE=1 cargo test -p halp-ext --test
   example_pack` rebuilds this example's wasm, file index and bundle with
   the test key.)

   ```
   halpd pack keygen --out my-pack.key        # keep this private
   halpd pack sign <folder> --key my-pack.key --out my.halppack
   halpd pack verify my.halppack --trust my-pack.pub   # the public key, saved to a file
   ```

   `keygen` prints the key's fingerprint and public key: publish them
   (users compare the fingerprint before trusting it). The private key
   file is mode 0600; back it up and never commit it.
5. Install: Settings > Packs > choose your `.halppack`, check the permission
   review (modules, calls, prompts), and for a pack signed with a key the
   user has not trusted yet, choose to trust the key. An update must be
   signed with the same key.

An unsigned pack can still install for data-only changes (roles, types and
prompts are not served over MCP for unsigned packs), but it cannot carry
modules or UI widgets, so it cannot ship this widget.

## What the pack can and cannot do

The transform and widget run as the actor `module:acme.platform-sre`: they
can only read `general`-class tasks, never write, never reach the network,
and never see private-class items. Prompts from this pack are served over
MCP only while the pack is enabled, labelled `[community: acme.platform-sre]`.
