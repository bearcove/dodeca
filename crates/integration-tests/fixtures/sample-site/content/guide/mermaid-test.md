+++
title = "Mermaid Test"
+++

# Mermaid Diagram Test

This page tests mermaid diagram rendering.

## Flowchart

```mermaid
flowchart LR
    A[Start] --> B{Decision}
    B -->|Yes| C[Do Something]
    B -->|No| D[Do Nothing]
    C --> E[End]
    D --> E
```

## Sequence Diagram

```mermaid
sequenceDiagram
    participant A as Alice
    participant B as Bob
    A->>B: Hello Bob!
    B-->>A: Hi Alice!
```
