+++
title = "Spec Traceability"
weight = 60
+++

dodeca supports [tracey](https://github.com/bearcove/tracey) requirement markers in markdown, letting you write specs that are tracked against your implementation.

## Syntax

Define a requirement with the tracey marker syntax:

```markdown
r[protocol.handshake]

The client MUST send a handshake message within 5 seconds of connecting.
```

This renders as a styled requirement block with an anchor, just like tracey's own documentation renders them.

## Use case

If your project uses tracey for spec coverage, you can write your specification as a dodeca site. The requirement IDs in your markdown are the same ones you reference in your source code with `// [impl protocol.handshake]` comments.

See the [tracey documentation](https://github.com/bearcove/tracey) for the full spec coverage workflow.
