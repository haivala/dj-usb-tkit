// Shared fake DOMTokenList for tests that build plain-object DOM stand-ins.
// toggle() matches real DOMTokenList semantics: with no `force`, it flips
// current membership and returns the new membership state; with a boolean
// `force`, it sets membership to match and returns `force`.
export function makeClassList() {
  const classes = new Set();
  return {
    add(name) { classes.add(name); },
    remove(name) { classes.delete(name); },
    contains(name) { return classes.has(name); },
    toggle(name, force) {
      if (typeof force === "boolean") {
        if (force) classes.add(name);
        else classes.delete(name);
        return force;
      }
      if (classes.has(name)) {
        classes.delete(name);
        return false;
      }
      classes.add(name);
      return true;
    }
  };
}
