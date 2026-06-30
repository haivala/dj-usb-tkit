import test from "node:test";
import assert from "node:assert/strict";
import {
  registerBackendJobEvents,
  unregisterBackendJobEvents,
  bindBeforeUnloadCleanup
} from "../components/playback/actions.mjs";

test("registerBackendJobEvents unregisters old listeners and stores new ones", async () => {
  let oldJobUnlistenCalls = 0;
  let oldPlaybackUnlistenCalls = 0;
  let newJobUnlistenCalls = 0;
  let newPlaybackUnlistenCalls = 0;

  const state = {
    unlistenJobEvent: () => { oldJobUnlistenCalls += 1; },
    unlistenPlaybackEvent: () => { oldPlaybackUnlistenCalls += 1; }
  };

  await registerBackendJobEvents(state, {
    isTauriRuntime: () => true,
    unregisterBackendJobEvents: () => unregisterBackendJobEvents(state, {}),
    getTauriEventListen: async () => async (eventName, handler) => {
      assert.equal(typeof handler, "function");
      if (eventName === "job:event") return () => { newJobUnlistenCalls += 1; };
      if (eventName === "playback:event") return () => { newPlaybackUnlistenCalls += 1; };
      throw new Error(`unexpected event ${eventName}`);
    },
    handleJobEvent: () => {},
    handlePlaybackEvent: () => {}
  });

  assert.equal(oldJobUnlistenCalls, 1);
  assert.equal(oldPlaybackUnlistenCalls, 1);
  assert.equal(typeof state.unlistenJobEvent, "function");
  assert.equal(typeof state.unlistenPlaybackEvent, "function");

  await unregisterBackendJobEvents(state, {});
  assert.equal(newJobUnlistenCalls, 1);
  assert.equal(newPlaybackUnlistenCalls, 1);
  assert.equal(state.unlistenJobEvent, null);
  assert.equal(state.unlistenPlaybackEvent, null);
});

test("beforeunload handler triggers backend unlisten cleanup", async () => {
  let beforeUnloadHandler = null;
  let unregisterCalls = 0;

  bindBeforeUnloadCleanup(
    {
      addEventListener: (name, handler) => {
        if (name === "beforeunload") beforeUnloadHandler = handler;
      }
    },
    async () => {
      unregisterCalls += 1;
    }
  );

  assert.equal(typeof beforeUnloadHandler, "function");
  beforeUnloadHandler();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(unregisterCalls, 1);
});
