// Command responses shaped like the backend's (backend/src/models.rs), for
// stubbed `ctx.command`s. The frontend reads them without re-checking their
// shape, so a stub must carry every field the struct always serializes.
// `overrides` replaces fields.

// `ValidateUsbRootData` (validate_usb_root): an initialized, writable stick.
export function validateUsbRootData(overrides = {}) {
  return {
    valid: true,
    hasWriteAccess: true,
    normalizedRoot: "/tmp/usb",
    hasVendorRoot: true,
    hasContents: true,
    hasPdb: true,
    hasEdb: true,
    warnings: [],
    ...overrides
  };
}

// `RunUsbDiagnosticsData` (run_usb_diagnostics).
export function usbDiagnosticsData(overrides = {}) {
  return {
    overallStatus: "PASS",
    overview: [],
    sectionGroups: [],
    playlistDetails: [],
    warnings: [],
    durationMs: 1,
    playlistUsbExportStatus: [],
    ...overrides
  };
}

// `RepairFixProposal`: one fix in a repair preview.
export function repairFix(id, overrides = {}) {
  return {
    id,
    title: id,
    description: "desc",
    supported: true,
    destructive: false,
    estimatedWrites: 0,
    estimatedDeletes: 0,
    alwaysApplied: false,
    bookkeeping: false,
    ...overrides
  };
}

// `RepairUsbDiagnosticsData` (repair_usb_diagnostics, preview or apply).
export function repairData(overrides = {}) {
  return {
    summary: "",
    overview: [],
    detectedIssues: [],
    proposedFixes: [],
    unsupportedItems: [],
    appliedFixes: [],
    skippedFixes: [],
    failedFixes: [],
    estimatedFileWrites: 0,
    estimatedFileDeletes: 0,
    warnings: [],
    durationMs: 1,
    engineLibraryEnabled: false,
    ...overrides
  };
}

// `UsbPlayerMenuItem`: one player-menu category.
export function playerMenuItem(kind, overrides = {}) {
  return {
    menuItemId: kind,
    kind,
    name: `Menu ${kind}`,
    isVisible: true,
    sequenceNo: null,
    origin: "both",
    removable: true,
    ...overrides
  };
}

// `UpdateUsbPlayerMenuConfigData` (get/update/sync player-menu config).
export function playerMenuConfig(overrides = {}) {
  return {
    updated: false,
    currentItems: [],
    availableItems: [],
    divergence: {
      inEdbVisibleOnly: [],
      inPdbOnly: [],
      orderMismatch: false,
      pdbMissingKinds: [],
      summary: "",
      canSync: false,
      canRestore: false
    },
    warnings: [],
    ...overrides
  };
}
