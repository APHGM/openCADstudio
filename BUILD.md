# Building ocs-pointcloud-clip

## Prerequisites

1. Install Rust: https://rustup.rs/
   - Windows: download rustup-init.exe and run it
   - Choose "1) Proceed with standard installation"

2. After install, open a new terminal and verify:
   ```
   rustc --version
   cargo --version
   ```

## Build

```
cd ocs-pointcloud-clip
cargo build --release
```

The compiled plugin will be at:
  `target\release\ocs_pointcloud_clip.dll`

## Install into Open CAD Studio

1. Create the plugin folder:
   `%APPDATA%\OpenCADStudio\plugins\com.geocentroid.pcclip\`

2. Copy these two files there:
   - `target\release\ocs_pointcloud_clip.dll`
   - `plugin.toml`

3. Restart Open CAD Studio.

4. The "Point Cloud" ribbon tab should appear with 4 buttons:
   - Clip Region (PC_CLIP)
   - Clip Back   (PC_BACK)
   - Release All (PC_RELEASE)
   - Clip Status (PC_STATUS)

## Workflow

1. Load an RCP file via External References.
2. Click **Clip Region** → pick two corners of the area you want to zoom into.
3. Click **Clip Region** again on a sub-region to drill deeper.
4. Click **Clip Back** to go up one level.
5. Click **Release All** to show the full point cloud.
6. Click **Clip Status** to see current depth in the command line.

## Troubleshooting compile errors

If `cargo build` reports errors about missing methods on `HostApi` (e.g. `run_command`,
`query_entities`, `begin_undo_group`), the OCS team may have changed method names since
this was written. Check:

  https://github.com/HakanSeven12/OpenCADStudio/blob/main/crates/ocs_plugin_api/src/host.rs

and update the method calls in `src/lib.rs` to match.

The likely culprits:
- `host.run_command(CommandRequest::Run { line })` — may be `host.execute(...)` or similar
- `host.query_entities(json)` — may need a different query struct
- `host.begin_undo_group / end_undo_group` — confirmed in architecture docs

If `EntityType` doesn't have `.handle()`, use `.handle` as a field directly.
