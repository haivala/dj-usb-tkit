import {
  convertFileSrc as tauriConvertFileSrc,
  invoke as tauriInvoke,
  isTauri as tauriIsTauri,
} from "@tauri-apps/api/core";
import { getVersion as tauriGetVersion } from "@tauri-apps/api/app";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { createApiClient } from "./api_client.mjs";
import { createAppContext } from "./app_context.mjs";
import { bindBeforeUnloadCleanup } from "./components/playback/actions.mjs";
import { initApp } from "./startup_bootstrap.mjs";
import { renderWaveformsIn } from "./waveform.mjs";

const ctx = createAppContext({
  ...createApiClient({ tauriInvoke, tauriIsTauri, tauriListen }),
  tauriConvertFileSrc,
  tauriIsTauri,
  tauriGetVersion,
  document,
  window,
  navigator,
  localStorage,
});

window.addEventListener("resize", () => {
  renderWaveformsIn(document);
});
bindBeforeUnloadCleanup(ctx);

initApp(ctx).catch((error) => {
  ctx.state.startupPhase = false;
  console.error(error);
  ctx.emitStatus(`Initialization failed: ${error.message}`);
});
