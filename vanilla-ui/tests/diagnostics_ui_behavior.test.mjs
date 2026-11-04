import test from "node:test";
import assert from "node:assert/strict";
import { clearUsbDiagnostics, hideUsbDiagnostics } from "../components/usb/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

// The real app context, with a stale report and an open, loading health card.
function makeDiagnosticsCtx() {
  const ctx = makeTestCtx();
  const { el } = ctx;
  el.usbHealthCard.setAttribute("open", "");
  el.usbHealthCard.classList.add("is-loading");
  el.usbHealthDot.classList.add("health-warn");
  el.diagSections.innerHTML = "<div>stale</div>";
  el.diagRepairFixes.innerHTML = "<div>stale fix</div>";
  el.diagOverallStatus.textContent = "WARN";
  el.diagDuration.textContent = "Completed in 1ms";
  el.diagRepairSummary.textContent = "stale summary";
  el.previewRepairsBtn.disabled = false;
  el.applyRepairsBtn.disabled = false;
  el.diagReportView.classList.add("hidden");
  el.diagRepairPanel.classList.remove("hidden");
  return ctx;
}

function assertDiagnosticsContentCleared(el) {
  for (const key of ["diagSections", "diagRepairFixes"]) {
    assert.equal(el[key].innerHTML, "");
  }
  for (const key of ["diagOverallStatus", "diagDuration", "diagRepairSummary"]) {
    assert.equal(el[key].textContent, "");
  }
  assert.equal(el.previewRepairsBtn.disabled, true);
  assert.equal(el.applyRepairsBtn.disabled, true);
  assert.equal(el.diagReportView.classList.contains("hidden"), false);
  assert.equal(el.diagRepairPanel.classList.contains("hidden"), true);
  assert.equal(el.usbHealthDot.classList.contains("health-warn"), false);
  assert.equal(el.usbHealthDot.dataset.tooltip, "USB health: unknown");
}

test("clearUsbDiagnostics and hideUsbDiagnostics blank content with different visibility effects", () => {
  for (const [action, cardHidden, cardOpen, loading] of [
    [clearUsbDiagnostics, false, true, true],
    [hideUsbDiagnostics, true, false, false]
  ]) {
    const ctx = makeDiagnosticsCtx();
    const { el } = ctx;
    el.usbDiagnosticsCard.classList.remove("hidden");
    action(ctx);
    assertDiagnosticsContentCleared(el);
    assert.equal(el.usbDiagnosticsCard.classList.contains("hidden"), cardHidden);
    assert.equal(el.usbHealthCard.open, cardOpen);
    assert.equal(el.usbHealthCard.classList.contains("is-loading"), loading);
  }
});
