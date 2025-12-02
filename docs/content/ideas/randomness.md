+++
title = "Randomness"
description = "Handling random values in incremental builds"
weight = 10
+++

**Status: Design idea, not yet implemented**

## Problem

Some site features need "randomness":
- "Random article" links
- Shuffled content blocks
- A/B test variants

But randomness breaks incremental builds - if outputs change every build, nothing is cached.

## Solution

**Dev mode**: Random values come from fixed input (a constant seed). They never change, so:
- Incremental builds work perfectly
- Live reload doesn't thrash
- Developers see consistent output

**Production builds**: Generate a fresh seed at build time. Random values actually vary:
- Each deployment gets different "random" picks
- Still deterministic within a single build (same seed = same output)

## Implementation Ideas

```rust
// In template context
fn random_seed(&self) -> u64 {
    if self.is_dev_mode {
        0xDEADBEEF  // Fixed seed for dev
    } else {
        self.build_seed  // Generated at build start
    }
}

// Template usage
{% set rng = random_seed() %}
{% set random_article = articles | shuffle(seed=rng) | first %}
```

This keeps the randomness as *input* to the build, making it cacheable.
