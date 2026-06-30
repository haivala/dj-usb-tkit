# Desktop Host (Tauri)

This host loads built frontend assets from `../vanilla-ui/dist` and backend commands from `../backend`.
JS analysis runtime dependencies live in `desktop/package.json`.

## Install JS Dependencies

```bash
cd desktop
npm install
```

## Run

```bash
cd ../vanilla-ui
npm run build
cd desktop/src-tauri
cargo run
```

## Run on Native Wayland

```bash
cd desktop
./run-wayland.sh
```

`run-wayland.sh` builds `vanilla-ui/dist` before launching Tauri so the app uses the
current frontend bundle.

## Essentia Runtime (Optional)

Essentia is optional. Users enable it from app settings and download required Essentia files on demand.

When Essentia is enabled, the app resolves the Node runtime in this order:

1. `DJTKIT_ESSENTIA_NODE` (explicit override)
2. bundled runtime candidates (if present in dev/custom builds):
   - `<resources>/bin/node` or `node.exe`
   - `<exe-dir>/node`
   - `desktop/runtime/bin/node` (dev fallback)
3. system `node` fallback

Runner script resolution order:

1. `DJTKIT_ESSENTIA_RUNNER` (explicit override)
2. `<resources>/scripts/essentia_runner.cjs`
3. `desktop/scripts/essentia_runner.cjs`

Default public releases do not ship a bundled Node runtime.
