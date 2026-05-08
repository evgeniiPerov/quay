---
name: web-accessibility
description: WCAG 2.2 AA accessibility rules for the Next.js web app.
paths:
  - "apps/web/**/*.tsx"
  - "apps/web/**/*.jsx"
  - "apps/web/**/*.css"
---

# Web Accessibility — `apps/web/`

Target: **WCAG 2.2 AA**. Enforced by axe in CI (see [`web-testing.md`](web-testing.md)).

## 1. Semantic HTML first

- One `<h1>` per page. Heading levels descend without skipping (`h1 → h2 → h3`).
- `<button>` for actions, `<a>` for navigation. Never the reverse.
- `<nav>`, `<main>`, `<footer>`, `<article>`, `<aside>` instead of `<div>` soup.
- Lists use `<ul>` / `<ol>` / `<li>`. Don't fake them with `<div>`.

## 2. ARIA is a last resort

- Reach for ARIA only when no semantic HTML element does the job.
- `role="button"` on a `<div>` is wrong — use `<button>`.
- `aria-label` on a `<button>` with visible text is wrong — the visible text is the label.
- Test with a real screen reader (VoiceOver on macOS, NVDA on Windows, Orca on Linux) before shipping anything that uses ARIA.

## 3. Keyboard

- Every interactive element reachable by `Tab`. Tab order matches visual order.
- Focus is always visible. Don't remove the focus ring without replacing it with a clearly distinguishable alternative.
- `Escape` closes modals and dropdowns. `Enter` / `Space` activates buttons.
- No keyboard traps. Modal focus is restored to the trigger element on close.
- Custom interactive widgets (combobox, listbox, tree) follow the [WAI-ARIA Authoring Practices](https://www.w3.org/WAI/ARIA/apg/) keyboard patterns. Or, better, use a battle-tested library (Radix, React Aria, shadcn).

## 4. Color + contrast

- Body text contrast ≥ 4.5:1. Large text (≥18pt or 14pt bold) ≥ 3:1.
- UI components and graphical objects ≥ 3:1 against adjacent colors.
- Don't convey information by color alone. Pair with icons, text, or patterns.
- Test in dark mode and light mode.

## 5. Forms

- Every input has a `<label>` (visible) or `aria-labelledby` (when label is visually hidden but exists).
- `placeholder` is **not** a label. It disappears on focus and has poor contrast by default.
- Errors associated via `aria-describedby` and `aria-invalid="true"`. Error text appears next to the field, not buried elsewhere.
- Required fields marked with both `required` attribute and visible "(required)" or asterisk + legend.
- `autocomplete` set on every personal-data field (`name`, `email`, `tel`, `street-address`, etc.).

## 6. Images + media

- Every `<Image>` / `<img>` has `alt`. Decorative images: `alt=""` (empty, not missing).
- Icon-only buttons: `aria-label` describes the action.
- Video has captions. Audio has a transcript.
- No autoplaying audio or video with sound.

## 7. Motion

- Respect `prefers-reduced-motion: reduce`. Disable parallax, large transitions, autoplay carousels.
- Animations under 5 seconds or pausable.
- No flashing content (>3 flashes per second) — seizure trigger.

## 8. Touch targets

- Minimum 24×24 CSS pixels (WCAG 2.2 AA). Recommended 44×44.
- Spacing between targets ≥ 8px.

## 9. Live regions

- Toast / notification regions: `role="status"` (polite) or `role="alert"` (assertive — sparingly).
- Don't fire `role="alert"` for routine UI changes; it interrupts the user.

## 10. Testing

- Lint at write time: `eslint-plugin-jsx-a11y` or Biome's a11y rules.
- Unit: `vitest-axe` on individual components.
- E2E: `@axe-core/playwright` on every page (see `web-testing.md`).
- Manual: keyboard-only walkthrough of the happy path before merging any UI change.
- Real screen reader test before any release that touches custom widgets.

## 11. Forbidden

- `tabindex` values > 0 (breaks tab order).
- `outline: none` without a replacement focus indicator.
- `aria-hidden="true"` on focusable elements.
- `<div onclick>` without role + keyboard handlers.
- Auto-focusing inputs on page load (jumps assistive tech).
- Color contrast checks bypassed because "the designer said so". The designer is wrong.

## See also

- [`anthropics/skills@frontend-design`](../skills/frontend-design/) — design system + a11y guidance
- [`vercel-labs/agent-skills@web-design-guidelines`](../skills/) (install if needed)
- [`chrome-devtools-mcp:a11y-debugging`](https://chromedevtools.github.io/) skill for axe debugging
