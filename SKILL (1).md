---
name: design-research
description: Directs the agent to ground design/UI decisions in real reference sources (shipped product patterns and award-tier motion design) instead of generic templates, before doing significant visual design or redesign work. Use before any multi-screen redesign, new feature UI, or when the user says the design looks generic/templated/boring and wants real inspiration, not just before writing CSS from scratch.
---

# Design Research Playbook

Before designing or redesigning a non-trivial screen, don't invent patterns from scratch or
default to generic SaaS-dashboard conventions. Ground the work in how real, shipped products
in this category solve the same problem.

## Reference sources and what each is actually good for

| Source | What it's for | When to use it |
|---|---|---|
| **Mobbin** (mobbin.com) | A searchable library of real, shipped mobile & web app UI screens by flow (onboarding, empty states, upload flows, settings) | Best source for this app specifically — search "video editor," "upload," "AI generation," "onboarding" to see how shipped products solve the same flows this app has |
| **Awwwards** (awwwards.com) | Award-tier marketing/product sites, strong on motion and interaction craft | Reference for the processing/progress view and any moment that should feel premium |
| **Godly** (godly.website) | Curated, motion-heavy landing and product page designs | Good source specifically for scroll/entrance motion ideas — pairs with the `scroll-and-motion-design` skill |
| **Land-book** (land-book.com) | Filterable gallery of landing pages by style/industry | Useful if/when this app gets a marketing site, less relevant to in-app screens |
| **Dribbble** / **Behance** | Broad visual/illustration direction, concept exploration | Use for visual direction and mood only — these are concept shots, not shipped, functioning UI, so don't copy interaction patterns from them uncritically |

## How to use this during a redesign task

1. Before writing CSS for a new or redesigned screen, briefly describe (in your own words,
   not by reproducing any source's copy or layout wholesale) what pattern real apps in this
   space use for the equivalent flow — e.g. "AI clipping tools typically show a staged
   progress indicator with named steps, not a bare spinner."
2. Cross-check against the `crayo-style-ui-review` skill's checklist, which already encodes
   the category-specific conventions (staged progress, scored clip cards, visual template
   gallery) — that skill is the distilled output of this kind of research for this app, so
   you don't need to re-derive it every time, only when working on something it doesn't
   already cover.
3. Never copy a specific screen pixel-for-pixel from a reference source, and never reproduce
   another product's copy, illustrations, or brand assets. Use these sources for pattern and
   interaction inspiration only, then execute it in this app's own design system (tokens
   defined in `styles.css`, per the `crayo-style-ui-review` skill).
4. If you don't have live web access in this environment, state that you're working from
   documented UI conventions for this product category rather than fetching current examples,
   so the user knows the difference.

## Anti-pattern to avoid

Don't default to generic "AI app" visual tropes (a single warm-cream-and-serif look, or a
near-black background with one neon accent, or interchangeable rounded SaaS cards with
identical shadows on everything) just because they're common in AI-generated interfaces.
Ground every choice in what this specific app does and who uses it, not in what's fastest to
generate.
