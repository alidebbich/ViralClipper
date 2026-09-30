---
name: scroll-and-motion-design
description: Playbook for implementing smooth scrolling, scroll-driven reveals, parallax, and UI motion/transitions in the app — which library to use for which job, concrete setup code, and performance/accessibility rules. Use whenever adding scroll effects, page/element transitions, hover/gesture animation, or any "make it feel smoother / more alive / more premium" request, even if the user doesn't name a specific library or effect.
---

# Scroll & Motion Implementation Playbook

This app runs inside a Tauri webview (system WebView2/WebKit) — it's a real browser engine,
so standard web animation libraries work normally. No special Tauri handling is needed for
motion/scroll code itself; only for anything that touches the filesystem or native APIs.

## Decision matrix — pick the right tool, don't reach for one library for everything

| Need | Use | Why |
|---|---|---|
| Buttery native-feel page scrolling | **Lenis** (`lenis` npm package) | Wraps native scroll, keeps `position: sticky`, anchor links, and accessibility working. A few KB, zero dependencies. |
| Scroll-driven pin/scrub/parallax sequences (complex, multi-step) | **GSAP + ScrollTrigger** | GSAP is 100% free including ScrollTrigger. Best for imperative, timeline-based choreography (parallax, pinning, scrubbing). |
| React component enter/exit, layout animation, gestures (drag, hover, tap) | **Motion** (the renamed/current Framer Motion, `npm i motion`) | Declarative — describe the end state, the library animates to it. Best fit for component-based UI state transitions. |
| Simple, tiny viewport-entry fades on scroll | **Motion's `whileInView`** or native CSS `animation-timeline` | Don't pull in GSAP for a simple fade-in-on-scroll; that's overkill. |
| Smooth list reordering (clip grid re-sorting by score, template list filtering) | **Motion `layout` prop**, or `@formkit/auto-animate` for a zero-config one-liner | Matches this app's clip-grid and template-gallery use cases directly. |

Only add a new dependency if nothing already in the project covers the need — check
`package.json` first.

## Wiring Lenis + GSAP ScrollTrigger together (if both are used)

```js
import Lenis from 'lenis'
import { gsap } from 'gsap'
import { ScrollTrigger } from 'gsap/ScrollTrigger'

gsap.registerPlugin(ScrollTrigger)

const lenis = new Lenis()
lenis.on('scroll', ScrollTrigger.update)
gsap.ticker.add((time) => lenis.raf(time * 1000))
gsap.ticker.lagSmoothing(0)
```

Include Lenis's recommended CSS (`import 'lenis/dist/lenis.css'`) or the app's scroll
containers will fight the library.

## Where this actually applies in this app

- The paste-link/import screen and clip-results grid are the highest-value places for
  smooth scroll and entrance motion — they're the first and most-viewed screens.
- The processing/progress view should use motion to make stage transitions feel alive
  (e.g. a stage label crossfading in), which also reinforces the staged-progress fix from
  the pipeline-debugging skill — motion here is functional, not decorative.
- The template gallery benefits from `layout` animation when filtering/reordering.

## Hard rules (do not skip these)

1. **Respect `prefers-reduced-motion`.** Wrap non-essential motion (parallax, scroll reveals)
   in a media query check and skip/simplify it for users who've opted out.
2. **Clean up on unmount.** Every Lenis instance, GSAP ScrollTrigger, and `requestAnimationFrame`
   loop must be destroyed/killed in the corresponding React `useEffect` cleanup function, or
   it leaks across tab switches in this app's 7-tab shell.
3. **One orchestrated moment beats scattered effects.** Per this project's design guidance
   (see the `crayo-style-ui-review` skill), don't add a fade-and-slide-up to every card by
   default — that reads as generic. Pick the 1-2 places motion genuinely helps (page load,
   stage transitions, clip grid population) and keep the rest calm.
4. **60fps or don't ship it.** Animate `transform`/`opacity` only where possible; avoid
   animating layout-triggering properties (`width`, `top`, `left`) in scroll-driven code.
5. **Test at real content volume.** Scroll/list animations that look fine with 3 mock clips
   can jank with 25+ real generated clips — verify against real pipeline output.
