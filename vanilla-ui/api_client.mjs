// Backend API client: Tauri invoke wrapper.

export function createApiClient({
  tauriInvoke,
  tauriIsTauri,
  tauriListen,
}) {
  function isTauriRuntime() {
    try {
      return !!tauriIsTauri();
    } catch (_) {
      return false;
    }
  }

  async function getTauriEventListen() {
    if (window.__TAURI__?.event?.listen) return window.__TAURI__.event.listen;
    return isTauriRuntime() ? tauriListen : null;
  }

  async function invoke(command, payload = {}) {
    if (isTauriRuntime()) {
      return tauriInvoke(command, payload);
    }

    if (window.__TAURI__?.core?.invoke) {
      return window.__TAURI__.core.invoke(command, payload);
    }

    return {
      ok: false,
      error: {
        code: "INTERNAL_ERROR",
        message: `No backend available for command: ${command}`,
      },
    };
  }

  async function command(commandName, request = null) {
    const payload = request === null ? {} : { request };
    const response = await invoke(commandName, payload);

    if (!response?.ok) {
      const info = response?.error || {};
      const error = new Error(info.message || `Command failed: ${commandName}`);
      // Carry the backend's structured error fields through so callers key off
      // `err.code` / `err.details` instead of matching the message string.
      if (info.code) error.code = String(info.code);
      if (info.details != null) error.details = info.details;
      throw error;
    }

    return response.data;
  }

  return { invoke, command, isTauriRuntime, getTauriEventListen };
}
