// Backend API client: Tauri invoke wrapper plus optional browser/dev mock.

export function createApiClient({
  tauriInvoke,
  tauriIsTauri,
  tauriListen,
  mockInvoke = null,
  loadMockInvoke = null,
}) {
  let mockInvokePromise = null;

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

  async function getMockInvoke() {
    if (typeof mockInvoke === "function") return mockInvoke;
    if (typeof loadMockInvoke !== "function") return null;
    if (!mockInvokePromise) {
      mockInvokePromise = Promise.resolve(loadMockInvoke()).then((loaded) => {
        if (typeof loaded === "function") return loaded;
        if (typeof loaded?.invoke === "function") return loaded.invoke;
        return null;
      });
    }
    return mockInvokePromise;
  }

  async function invoke(command, payload = {}) {
    if (isTauriRuntime()) {
      return tauriInvoke(command, payload);
    }

    if (window.__TAURI__?.core?.invoke) {
      return window.__TAURI__.core.invoke(command, payload);
    }

    const fallback = await getMockInvoke();
    if (fallback) return fallback(command, payload);

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
