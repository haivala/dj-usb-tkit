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
      const msg = response?.error?.message || `Command failed: ${commandName}`;
      throw new Error(msg);
    }

    return response.data;
  }

  return { invoke, command, isTauriRuntime, getTauriEventListen };
}
