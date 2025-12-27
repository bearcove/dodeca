+++
title = "ASCII Diagrams"
description = "Render ASCII art diagrams as SVG"
weight = 45
+++

Dodeca supports rendering ASCII art diagrams as inline SVG using the [aasvg](https://crates.io/crates/aasvg) crate. This lets you create diagrams in plain text that render beautifully in your documentation.

## Usage

Use a fenced code block with the language identifier `aa`:

````markdown
```aa
    +-----+     +-----+
    |  A  |---->|  B  |
    +-----+     +-----+
```
````

This renders as:

```aa
    +-----+     +-----+
    |  A  |---->|  B  |
    +-----+     +-----+
```

## Examples

### Flowchart

```aa
    +--------+
    | Start  |
    +---+----+
        |
        v
    +---+----+
    | Step 1 |
    +---+----+
        |
        v
    +---+----+     +--------+
    | Check  +---->| Branch |
    +---+----+     +--------+
        |
        v
    +---+----+
    |  End   |
    +--------+
```

### Sequence Diagram

```aa
    Client              Server              Database
      |                   |                   |
      |  HTTP Request     |                   |
      +------------------>|                   |
      |                   |   SQL Query       |
      |                   +------------------>|
      |                   |                   |
      |                   |<------------------+
      |                   |   Result Set      |
      |<------------------+                   |
      |   HTTP Response   |                   |
      |                   |                   |
```

### Architecture Diagram

```aa
    +---------------+     +---------------+
    |   Frontend    |     |    Backend    |
    |  (Browser)    |<--->|   (Server)    |
    +---------------+     +-------+-------+
                                  |
                                  v
                          +-------+-------+
                          |   Database    |
                          +---------------+
```

### Box Drawing

```aa
    +-----------+-------------------+
    |  Header   |      Value        |
    +-----------+-------------------+
    |  Name     |  Alice            |
    |  Score    |  100              |
    |  Status   |  Active           |
    +-----------+-------------------+
```

## Supported Characters

The aasvg renderer supports standard ASCII box drawing:

- Corners: `+`
- Horizontal lines: `-`
- Vertical lines: `|`
- Arrows: `<`, `>`, `^`, `v`
- Arrow lines: `->`, `<-`, `-->`, `<--`
- Text: any alphanumeric characters

## Tips

- Use consistent spacing for alignment
- Add whitespace around diagrams for better rendering
- Keep diagrams simple and focused
- Use arrows to show direction and flow
