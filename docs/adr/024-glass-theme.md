# ADR-024: Glass theme

- **Status:** Accepted (extends ADR-019; ADR-027 limits glass to what floats)
- **Date:** 2026-11-24

## Context
The user asked for a "liquid glass" look. Glass costs GPU time, and translucent text
surfaces can become unreadable.

## Decision
- **CSS only**: `backdrop-filter` on a few fixed surfaces (sidebar, toolbar, status bar,
  details, dialogs, menus), never on list rows. The toolbar and status bar float over the
  list, so blur is used where content actually moves behind it.
- **Variables** define every surface; light, dark and the solid fallback differ only there.
- **Solid fallback**: a switch in Settings → Appearance, and automatically for
  `prefers-reduced-transparency` and `prefers-contrast: more`; forced colors keep borders.
- Text-heavy overlays are nearly opaque. Stripes mark work that is not downloading.
- No refraction (SVG displacement in `backdrop-filter`): Chromium only, and expensive.
- No native window material (Mica/acrylic): it differs per OS and needs a transparent
  window.

## Consequences
- The strict CSP is unchanged (no inline styles).
- Measured in headless Edge (the WebView2 engine) with 509 rows: median 7 ms and worst
  8 ms per frame while scrolling. Software-rendered WebKitGTK may be slower; the switch
  is for that.
