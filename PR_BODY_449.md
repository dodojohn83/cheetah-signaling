## Summary

`OwnerCommandHandler::handle_plugin_command` was locking `PluginHost` inside a `tokio::sync::Mutex` and holding the guard across `await` while calling `host.handle_command(...)`. Since `PluginHost::handle_command` only needs `&self` and the host is fully built before the worker starts, the lock was unnecessary and serialised all plugin-bound commands.

Remove the `Mutex` wrapping:

- `OwnerCommandHandler` now stores `Arc<PluginHost>` instead of `Arc<Mutex<PluginHost>>`.
- `handle_plugin_command` reads the plugin name and calls `self.plugin_host.handle_command(...).await` directly, without holding any guard over the await point.
- `ProcessAssembly` wraps the built `PluginHost` in `Arc` once after activation.
- `ApiState.plugin_host` changes type accordingly.
- Added a compile-time `Send + Sync` assertion for `OwnerCommandHandler` so the invariant required for lock-free sharing remains checked by CI.
