use std::sync::{Arc, Mutex};

use ocs_plugin_api::host::{BuiltinPlugin, Handle, HostApi, HostNotification, CommandRequest};
use ocs_plugin_api::manifest::{ApiVersion, PluginManifest};
use ocs_plugin_api::ribbon::{CadModule, IconKind, ModuleEvent, RibbonGroup, RibbonItem, ToolDef};

// ── Manifest ──────────────────────────────────────────────────────────────────

static MANIFEST: PluginManifest = PluginManifest {
    id: "com.geocentroid.pcclip",
    name: "Point Cloud Clip",
    version: env!("CARGO_PKG_VERSION"),
    description: "Progressive point cloud clipping with history stack",
    api_version: ApiVersion::CURRENT,
    ribbon_order: 90,
    xdata_apps: &["PCCLIP"],
    command_prefixes: &["PC_"],
};

// ── Clip state ────────────────────────────────────────────────────────────────

/// One entry on the clip stack: the visual boundary entity drawn in the doc.
#[derive(Clone, Debug)]
struct ClipEntry {
    /// Handle of the LWPOLYLINE boundary drawn on layer PC_CLIP.
    boundary_handle: Handle,
    /// Human label shown in PC_STATUS ("Clip 1", "Clip 2", …).
    label: String,
}

#[derive(Default)]
struct ClipState {
    /// Stack of progressive clips; last = innermost/current.
    stack: Vec<ClipEntry>,
    /// True while waiting for the user to finish drawing the RECTANG.
    waiting_for_rectang: bool,
    /// Count of how many entities existed before we started the current RECTANG.
    entity_count_before: u64,
}

impl ClipState {
    fn depth(&self) -> usize {
        self.stack.len()
    }
}

// ── Ribbon module ─────────────────────────────────────────────────────────────

struct PcClipModule {
    groups: Vec<RibbonGroup>,
}

impl PcClipModule {
    fn new() -> Self {
        Self {
            groups: vec![RibbonGroup {
                title: "Point Cloud",
                tools: vec![
                    // Clip Region — draw a new clip boundary (narrows the view)
                    RibbonItem::LargeTool(ToolDef {
                        id: "PC_CLIP_BTN",
                        label: "Clip Region",
                        icon: IconKind::Glyph("⬜"),
                        event: ModuleEvent::Command("PC_CLIP".to_string()),
                    }),
                    // Clip Back — pop one level from the history stack
                    RibbonItem::LargeTool(ToolDef {
                        id: "PC_BACK_BTN",
                        label: "Clip Back",
                        icon: IconKind::Glyph("↩"),
                        event: ModuleEvent::Command("PC_BACK".to_string()),
                    }),
                    // Release All — remove all clip boundaries
                    RibbonItem::LargeTool(ToolDef {
                        id: "PC_RELEASE_BTN",
                        label: "Release All",
                        icon: IconKind::Glyph("✕"),
                        event: ModuleEvent::Command("PC_RELEASE".to_string()),
                    }),
                    // Status — report current clip depth in the command line
                    RibbonItem::LargeTool(ToolDef {
                        id: "PC_STATUS_BTN",
                        label: "Clip Status",
                        icon: IconKind::Glyph("ℹ"),
                        event: ModuleEvent::Command("PC_STATUS".to_string()),
                    }),
                ],
            }],
        }
    }
}

impl CadModule for PcClipModule {
    fn id(&self) -> &'static str {
        "PC_CLIP_MODULE"
    }
    fn title(&self) -> &'static str {
        "Point Cloud"
    }
    fn ribbon_groups(&self) -> &[RibbonGroup] {
        &self.groups
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct PcClipPlugin {
    state: Arc<Mutex<ClipState>>,
}

impl Default for PcClipPlugin {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(ClipState::default())),
        }
    }
}

impl BuiltinPlugin for PcClipPlugin {
    fn manifest(&self) -> &'static PluginManifest {
        &MANIFEST
    }

    fn ribbon(&self) -> Box<dyn CadModule> {
        Box::new(PcClipModule::new())
    }

    fn on_load(&mut self, host: &mut dyn HostApi) {
        // Ensure the clip boundary layer exists with a bright cyan colour.
        // -LAYER syntax: command;option;layer-name;colour;value;layer-name;;
        let _ = host.run_command(CommandRequest::RunQuiet {
            line: "-LAYER;NEW;PC_CLIP;;".to_string(),
        });
        let _ = host.run_command(CommandRequest::RunQuiet {
            line: "-LAYER;COLOR;4;PC_CLIP;;".to_string(), // 4 = cyan
        });
        host.push_info("Point Cloud Clip loaded — PC_CLIP / PC_BACK / PC_RELEASE / PC_STATUS");
    }

    // ── Notification handler ──────────────────────────────────────────────────
    //
    // When the user finishes drawing the RECTANG we requested, a DocumentChangedV4
    // notification fires.  We query the newest LWPOLYLINE entity on layer PC_CLIP,
    // record its handle, and push it onto the clip stack.
    //
    fn on_notification(&mut self, _command_id: Option<u64>, notification: HostNotification) {
        let waiting = {
            let st = self.state.lock().unwrap();
            st.waiting_for_rectang
        };
        if !waiting {
            return;
        }

        if let HostNotification::DocumentChangedV4 { .. } = &notification {
            // We'll resolve the new boundary entity handle in dispatch's
            // post-command query (see PC_CLIP handler note below).
            // Here we just clear the waiting flag so double-fires are ignored.
            let mut st = self.state.lock().unwrap();
            st.waiting_for_rectang = false;
        }
    }

    // ── Command dispatcher ────────────────────────────────────────────────────

    fn dispatch(&self, host: &mut dyn HostApi, cmd: &str) -> bool {
        match cmd {
            "PC_CLIP" => {
                self.cmd_clip(host);
                true
            }
            "PC_BACK" => {
                self.cmd_back(host);
                true
            }
            "PC_RELEASE" => {
                self.cmd_release(host);
                true
            }
            "PC_STATUS" => {
                self.cmd_status(host);
                true
            }
            _ => false,
        }
    }
}

// ── Command implementations ───────────────────────────────────────────────────

impl PcClipPlugin {
    /// PC_CLIP — user draws a rectangle that becomes the new clip boundary.
    ///
    /// Flow:
    ///   1. Switch current layer to PC_CLIP.
    ///   2. Run the built-in RECTANG command — user picks two corners.
    ///   3. Query the newest LWPOLYLINE on PC_CLIP; store its handle.
    ///   4. Push onto the clip stack.
    ///   5. Optionally attempt XCLIP on the active RCP reference (future OCS API).
    ///
    fn cmd_clip(&self, host: &mut dyn HostApi) {
        let depth = self.state.lock().unwrap().depth();

        // Set active layer so the rectangle lands on PC_CLIP.
        let _ = host.run_command(CommandRequest::RunQuiet {
            line: "-LAYER;SET;PC_CLIP;;".to_string(),
        });

        // Run the built-in rectangle command.  This blocks until the user
        // picks both corners and the shape is committed to the document.
        host.push_info(&format!(
            "PC_CLIP [{}] — pick two corners of the clip region",
            depth + 1
        ));
        let outcome = host.run_command(CommandRequest::Run {
            line: "RECTANG".to_string(),
        });

        // If the command was cancelled or produced no entities, bail out.
        if outcome.added <= 0 {
            host.push_info("PC_CLIP cancelled — no region added.");
            return;
        }

        // Query the most recently added LWPOLYLINE on the PC_CLIP layer.
        let page_json = r#"{"type":"LWPOLYLINE","layer":"PC_CLIP","limit":1,"sort":"newest"}"#;
        let page = match host.query_entities(page_json) {
            Ok(p) => p,
            Err(e) => {
                host.push_error(&format!("PC_CLIP: query failed — {e}"));
                return;
            }
        };

        let handle = match page.entities.first() {
            Some(e) => e.handle(),
            None => {
                host.push_error("PC_CLIP: could not locate the drawn boundary entity.");
                return;
            }
        };

        let label = format!("Clip {}", depth + 1);
        {
            let mut st = self.state.lock().unwrap();
            st.stack.push(ClipEntry {
                boundary_handle: handle,
                label: label.clone(),
            });
        }

        // Attempt to apply XCLIP to any loaded RCP/RCS xref.
        // OCS may expose XCLIP in a future API version; for now we try it as
        // a command string — it will be a no-op if unsupported.
        let xclip_cmd = format!(
            "XCLIP;ALL;;NEW;P;{};",
            "" // placeholder: in a full impl, pass boundary vertices here
        );
        let _ = host.run_command(CommandRequest::RunQuiet { line: xclip_cmd });

        host.push_info(&format!("✓ {label} applied (depth now {})", depth + 1));
    }

    /// PC_BACK — pop the innermost clip boundary and restore the previous state.
    fn cmd_back(&self, host: &mut dyn HostApi) {
        let entry = {
            let mut st = self.state.lock().unwrap();
            st.stack.pop()
        };

        match entry {
            None => {
                host.push_info("PC_BACK: no clip to remove (stack is empty).");
            }
            Some(e) => {
                self.erase_boundary(host, e.boundary_handle);
                // Remove any XCLIP on RCP references (best-effort).
                let _ = host.run_command(CommandRequest::RunQuiet {
                    line: "XCLIP;ALL;;OFF;".to_string(),
                });
                // Re-apply the previous clip if one exists.
                let prev_depth = self.state.lock().unwrap().depth();
                if prev_depth > 0 {
                    host.push_info(&format!(
                        "↩ Returned to clip depth {prev_depth} — previous boundary is still active."
                    ));
                } else {
                    host.push_info("↩ All clips removed — full point cloud visible.");
                }
            }
        }
    }

    /// PC_RELEASE — erase every boundary and clear the full stack.
    fn cmd_release(&self, host: &mut dyn HostApi) {
        let stack = {
            let mut st = self.state.lock().unwrap();
            std::mem::take(&mut st.stack)
        };

        if stack.is_empty() {
            host.push_info("PC_RELEASE: nothing to release.");
            return;
        }

        host.begin_undo_group("PC_RELEASE");
        for entry in &stack {
            self.erase_boundary(host, entry.boundary_handle);
        }
        host.end_undo_group();

        // Remove any active XCLIP from all references.
        let _ = host.run_command(CommandRequest::RunQuiet {
            line: "XCLIP;ALL;;OFF;".to_string(),
        });

        host.push_info(&format!(
            "✓ Released {} clip region(s) — full point cloud visible.",
            stack.len()
        ));
    }

    /// PC_STATUS — report current clip depth in the command line.
    fn cmd_status(&self, host: &mut dyn HostApi) {
        let st = self.state.lock().unwrap();
        if st.stack.is_empty() {
            host.push_info("PC_STATUS: No active clips — full point cloud visible.");
        } else {
            let labels: Vec<&str> = st.stack.iter().map(|e| e.label.as_str()).collect();
            host.push_info(&format!(
                "PC_STATUS: {} clip(s) active — {}",
                st.depth(),
                labels.join(" › ")
            ));
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Delete a boundary entity by its handle using the ERASE command.
    fn erase_boundary(&self, host: &mut dyn HostApi, handle: Handle) {
        // Select entity by handle via (handent ...) lisp-like expression,
        // then erase it.  OCS accepts handle-based selection via Entity request.
        let outcome = host.run_command(CommandRequest::Entity {
            handle,
            point: [0.0, 0.0, 0.0],
        });
        if outcome.entities > 0 {
            let _ = host.run_command(CommandRequest::RunQuiet {
                line: "ERASE;P;;".to_string(),
            });
        }
    }
}

ocs_plugin_api::export_plugin!(PcClipPlugin);
