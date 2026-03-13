//! Hot reload support for external presets.
//!
//! Watches `~/.local/share/mcp-rtk/presets/` for file changes and atomically
//! rebuilds the [`FilterEngine`] when presets are added, modified, or removed.
//!
//! The rebuild is debounced (500 ms) to coalesce rapid successive writes
//! (e.g. editor save + rename). In-flight requests continue using the previous
//! engine while new requests pick up the updated one — zero downtime, no locks
//! on the hot path.
//!
//! # Architecture
//!
//! ```text
//! notify::Watcher ──event──▶ mpsc channel ──▶ tokio task (debounce + rebuild)
//!                                                      │
//!                                              ArcSwap<FilterEngine>.store()
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::config::Config;
use crate::filter::FilterEngine;

/// A hot-reloading wrapper around [`FilterEngine`].
///
/// Holds the file watcher alive and provides lock-free access to the current
/// engine via [`ArcSwap`]. Drop the `HotReloader` to stop watching.
///
/// # Examples
///
/// ```no_run
/// # use mcp_rtk::hot_reload::HotReloader;
/// # async fn example() -> anyhow::Result<()> {
/// let reloader = HotReloader::start(
///     vec!["npx".into(), "@nicepkg/gitlab-mcp".into()],
///     None,
///     None,
/// )?;
/// let engine = reloader.engine();
/// // engine.load() returns the current FilterEngine
/// # Ok(())
/// # }
/// ```
pub struct HotReloader {
    engine: Arc<ArcSwap<FilterEngine>>,
    // Held to keep the watcher alive; dropped when HotReloader is dropped.
    _watcher: RecommendedWatcher,
}

impl HotReloader {
    /// Start watching external presets and return the hot-reloadable engine.
    ///
    /// Builds the initial [`FilterEngine`] from the given arguments, then
    /// spawns a background tokio task that rebuilds it whenever preset files
    /// change on disk.
    ///
    /// # Arguments
    ///
    /// * `upstream_args` — The upstream MCP command (e.g. `["npx", "gitlab-mcp"]`).
    /// * `config_path` — Optional user config file path.
    /// * `preset_override` — Optional `--preset` override name.
    ///
    /// # Errors
    ///
    /// Returns an error if the initial config cannot be built or the file
    /// watcher cannot be created.
    pub fn start(
        upstream_args: Vec<String>,
        config_path: Option<PathBuf>,
        preset_override: Option<String>,
    ) -> anyhow::Result<Self> {
        // Build initial engine
        let upstream_refs: Vec<&str> = upstream_args.iter().map(|s| s.as_str()).collect();
        let config = Config::build(
            &upstream_refs,
            config_path.as_deref(),
            preset_override.as_deref(),
        )?;
        let engine = Arc::new(FilterEngine::new(Arc::new(config)));
        let swappable = Arc::new(ArcSwap::from(engine));

        // Channel to bridge notify (std thread) → tokio task
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        // Create file watcher
        let mut watcher =
            notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    if event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove() {
                        let _ = tx.send(());
                    }
                }
            })?;

        // Watch the external presets directory
        if let Ok(dir) = crate::config::external_presets_dir() {
            if let Err(e) = watcher.watch(&dir, RecursiveMode::NonRecursive) {
                tracing::warn!("Failed to watch presets dir: {e}");
            } else {
                tracing::info!("Watching external presets: {}", dir.display());
            }
        }

        // Also watch the user config file if one was provided
        if let Some(ref path) = config_path {
            if let Err(e) = watcher.watch(path, RecursiveMode::NonRecursive) {
                tracing::warn!("Failed to watch config file: {e}");
            }
        }

        // Spawn debounced reload task.
        // Shutdown: when HotReloader is dropped, _watcher is dropped, which
        // drops the event handler closure holding `tx`. This closes the channel,
        // causing `rx.recv()` to return `None` and breaking the loop.
        let swappable_clone = swappable.clone();
        tokio::spawn(async move {
            loop {
                // Wait for first event
                if rx.recv().await.is_none() {
                    break;
                }
                // Debounce: drain events for 500ms
                tokio::time::sleep(Duration::from_millis(500)).await;
                while rx.try_recv().is_ok() {}

                // Rebuild config + engine
                let refs: Vec<&str> = upstream_args.iter().map(|s| s.as_str()).collect();
                match Config::build(&refs, config_path.as_deref(), preset_override.as_deref()) {
                    Ok(config) => {
                        let new_engine = Arc::new(FilterEngine::new(Arc::new(config)));
                        swappable_clone.store(new_engine);
                        tracing::info!("Hot-reloaded filter engine with updated presets");
                    }
                    Err(e) => {
                        tracing::warn!("Hot-reload failed, keeping previous config: {e}");
                    }
                }
            }
        });

        Ok(Self {
            engine: swappable,
            _watcher: watcher,
        })
    }

    /// Get a reference to the shared, atomically-swappable engine.
    ///
    /// Use `engine.load()` to get the current [`FilterEngine`] for a request.
    pub fn engine(&self) -> &Arc<ArcSwap<FilterEngine>> {
        &self.engine
    }
}
