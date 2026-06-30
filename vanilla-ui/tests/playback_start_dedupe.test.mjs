import test from "node:test";
import assert from "node:assert/strict";

test("start-play dedupe promise pattern executes only one start", async () => {
  const state = { playbackStartPromise: null };
  let startCalls = 0;

  async function playTrackFromOriginLikeMain() {
    if (state.playbackStartPromise) {
      return state.playbackStartPromise;
    }
    const run = (async () => {
      startCalls += 1;
      await new Promise((resolve) => setTimeout(resolve, 15));
      return "ok";
    })();
    state.playbackStartPromise = run;
    try {
      return await run;
    } finally {
      if (state.playbackStartPromise === run) {
        state.playbackStartPromise = null;
      }
    }
  }

  const [a, b, c] = await Promise.all([
    playTrackFromOriginLikeMain(),
    playTrackFromOriginLikeMain(),
    playTrackFromOriginLikeMain()
  ]);

  assert.equal(a, "ok");
  assert.equal(b, "ok");
  assert.equal(c, "ok");
  assert.equal(startCalls, 1);
  assert.equal(state.playbackStartPromise, null);
});
