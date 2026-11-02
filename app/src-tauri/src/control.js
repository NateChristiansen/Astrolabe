// The page side of the test control, sent in by the test build for each ask. It is in no
// normal build and not in the page's own bundle. It finds real elements by what they show and
// dispatches real events at them. It calls nothing of the app's.
(ask) => {
  const shown = (el) => el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";
  const words = (text) => (text ?? "").replace(/\s+/g, " ").trim();
  const nameOf = (el) =>
    words(el.getAttribute("aria-label") || el.labels?.[0]?.innerText || el.innerText || el.placeholder);
  const roleOf = (el) =>
    el.getAttribute("role") ||
    { BUTTON: "button", SELECT: "combobox", TEXTAREA: "textbox", SUMMARY: "button", A: "link" }[el.tagName] ||
    (el.type === "radio" ? "radio" : el.type === "checkbox" ? "checkbox" : "textbox");
  const enabled = (el) =>
    !el.disabled && !el.readOnly && !el.closest("fieldset[disabled]") && el.getAttribute("aria-disabled") !== "true";
  const controls = () =>
    [...document.querySelectorAll("button,input,select,textarea,summary,a[href],[role=button]")].filter(shown);
  const describe = (el) => ({
    role: roleOf(el),
    name: nameOf(el),
    enabled: enabled(el),
    ...("value" in el && el.tagName !== "BUTTON" ? { value: el.value } : {}),
    ...(el.type === "radio" ? { checked: el.checked } : {}),
    ...(el.tagName === "SELECT" ? { options: [...el.options].map((one) => one.value) } : {}),
  });
  const find = ({ name, role }) => {
    const all = controls().filter((el) => !role || roleOf(el) === role);
    const exact = all.filter((el) => nameOf(el) === name);
    const found = exact.length ? exact : all.filter((el) => nameOf(el).includes(name));
    const list = controls().map((el) => `${roleOf(el)} "${nameOf(el)}"`).join(", ");
    if (found.length === 0) throw new Error(`nothing named "${name}" is showing. Showing: ${list}`);
    if (found.length > 1) throw new Error(`${found.length} things match "${name}". Showing: ${list}`);
    if (!enabled(found[0])) throw new Error(`"${nameOf(found[0])}" is disabled`);
    found[0].scrollIntoView({ block: "nearest" });
    return found[0];
  };
  const click = (el) => {
    const box = el.getBoundingClientRect();
    const [clientX, clientY] = [box.left + box.width / 2, box.top + box.height / 2];
    const hit = document.elementFromPoint(clientX, clientY);
    if (!hit || !(el.contains(hit) || [...(el.labels ?? [])].some((label) => label.contains(hit)))) {
      throw new Error(`"${nameOf(el)}" is covered by ${hit?.tagName ?? "nothing"}`);
    }
    const at = { bubbles: true, cancelable: true, composed: true, clientX, clientY, button: 0, view: window };
    el.dispatchEvent(new PointerEvent("pointerdown", at));
    el.dispatchEvent(new MouseEvent("mousedown", at));
    el.focus();
    el.dispatchEvent(new PointerEvent("pointerup", at));
    el.dispatchEvent(new MouseEvent("mouseup", at));
    el.dispatchEvent(new MouseEvent("click", at));
    return describe(el);
  };
  const type = (el, text) => {
    if (el.tagName === "SELECT" && ![...el.options].some((one) => one.value === text)) {
      throw new Error(`"${nameOf(el)}" has no choice "${text}"`);
    }
    if (!["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName)) throw new Error(`"${nameOf(el)}" takes no text`);
    el.focus();
    // Through the element's own setter, so the page's framework sees a change.
    Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), "value").set.call(el, text);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return describe(el);
  };
  try {
    if (ask.do === "read") return { ok: true, text: document.body.innerText, controls: controls().map(describe) };
    if (ask.do === "click") return { ok: true, clicked: click(find(ask)) };
    if (ask.do === "type") return { ok: true, typed: type(find(ask), ask.text) };
    return { ok: false, why: `astrolabe-test-control has no ask: ${ask.do}` };
  } catch (why) {
    return { ok: false, why: String(why.message ?? why) };
  }
}
