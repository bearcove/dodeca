+++
title = "Dual Cache Architecture"
weight = 31
+++

dodeca uses two caches: Salsa for the computation graph, and a content-addressed store for large blobs.

Salsa tracks dependencies and stores small outputs like parsed frontmatter, rendered HTML, and template ASTs. It stays in memory during a session and persists between runs.

Large outputs—processed images, subsetted fonts, any binary blob—go to a separate content-addressed store backed by [CanopyDB](https://github.com/bearcove/canopydb). Files are named by content hash, so identical content is never stored twice.

When you edit a markdown file, Salsa invalidates just the affected queries. The rendered HTML recomputes, but images with unchanged content skip reprocessing entirely—their hash is already in the CAS.

On startup, both caches load from disk. Most queries can reuse cached results; only changed inputs should trigger recomputation. A “cold” build can still be quicker if you’ve built the same project before.
