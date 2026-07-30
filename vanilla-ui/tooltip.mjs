const SHOW_DELAY_MS = 150;
const VIEWPORT_MARGIN = 8;
const GAP = 6;

function closestTooltipTarget(node) {
  return typeof node?.closest === "function" ? node.closest("[data-tooltip]") : null;
}

export function initTooltips({ document, window }) {
  let tooltipEl = null;
  let activeTarget = null;
  let showTimer = null;

  function ensureTooltipEl() {
    if (tooltipEl) return tooltipEl;
    tooltipEl = document.createElement("div");
    tooltipEl.id = "app-tooltip";
    tooltipEl.className = "app-tooltip";
    tooltipEl.setAttribute("role", "tooltip");
    document.body.appendChild(tooltipEl);
    return tooltipEl;
  }

  function positionTooltip(target) {
    const el = ensureTooltipEl();
    const targetRect = target.getBoundingClientRect();
    const tipRect = el.getBoundingClientRect();

    let top = targetRect.top - tipRect.height - GAP;
    if (top < VIEWPORT_MARGIN) {
      top = targetRect.bottom + GAP;
    }

    let left = targetRect.left + targetRect.width / 2 - tipRect.width / 2;
    const maxLeft = window.innerWidth - tipRect.width - VIEWPORT_MARGIN;
    left = Math.max(VIEWPORT_MARGIN, Math.min(left, maxLeft));

    el.style.top = `${Math.round(top)}px`;
    el.style.left = `${Math.round(left)}px`;
  }

  function showTooltip(target) {
    const text = target.dataset?.tooltip;
    if (!text) return;
    activeTarget = target;
    const el = ensureTooltipEl();
    el.textContent = text;
    el.classList.add("app-tooltip--visible");
    positionTooltip(target);
    target.setAttribute("aria-describedby", "app-tooltip");
  }

  function hideTooltip() {
    if (showTimer !== null) {
      window.clearTimeout(showTimer);
      showTimer = null;
    }
    if (activeTarget) {
      activeTarget.removeAttribute("aria-describedby");
      activeTarget = null;
    }
    tooltipEl?.classList.remove("app-tooltip--visible");
  }

  function scheduleShow(target) {
    if (showTimer !== null) {
      window.clearTimeout(showTimer);
    }
    showTimer = window.setTimeout(() => {
      showTimer = null;
      showTooltip(target);
    }, SHOW_DELAY_MS);
  }

  document.addEventListener("mouseover", (event) => {
    const target = closestTooltipTarget(event.target);
    if (!target || target === activeTarget) return;
    scheduleShow(target);
  });

  document.addEventListener("mouseout", (event) => {
    const target = closestTooltipTarget(event.target);
    if (!target) return;
    const related = event.relatedTarget;
    if (related && target.contains?.(related)) return;
    hideTooltip();
  });

  document.addEventListener("focusin", (event) => {
    const target = closestTooltipTarget(event.target);
    if (!target) return;
    showTooltip(target);
  });

  document.addEventListener("focusout", (event) => {
    const target = closestTooltipTarget(event.target);
    if (!target) return;
    hideTooltip();
  });

  document.addEventListener("scroll", () => hideTooltip(), true);

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") hideTooltip();
  });
}
