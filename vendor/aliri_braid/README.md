# aliri_braid

[![Build Status](https://github.com/neoeinstein/aliri_braid/actions/workflows/rust.yml/badge.svg?branch=main&event=push)](https://github.com/neoeinstein/aliri_braid)

Improve and strengthen your strings

Strongly-typed APIs reduce errors and confusion over passing around un-typed strings.
Braid helps in that endeavor by making it painless to create wrappers around your
string values, ensuring that you use them in the right way every time.

Examples of the documentation and implementations provided for braids are available
below and in the [`aliri_braid_examples`] crate documentation.

[`aliri_braid_examples`]: https://docs.rs/aliri_braid_examples/

## Usage

A braid is created by attaching `#[braid]` to a struct definition. The macro will take
care of automatically updating the representation of the struct to wrap a string and
generate the borrowed form of the strong type.

```rust
use aliri_braid::braid;

#[braid]
pub struct DatabaseName;
```

Braids of custom string types are also supported, so long as they implement a set of
expected traits. If not specified, the type named `String` in the current namespace
will be used.

```rust
use aliri_braid::braid;
use compact_str::CompactString as String;

#[braid]
pub struct UserId;
```

Once created, braids can be passed around as strongly-typed, immutable strings.

```rust
fn take_strong_string(n: DatabaseName) {}
fn borrow_strong_string(n: &DatabaseNameRef) {}

let owned = DatabaseName::new(String::from("mongo"));
borrow_strong_string(&owned);
take_strong_string(owned);
```

A braid can also be untyped for use in stringly-typed interfaces.

```rust
fn take_raw_string(s: String) {}
fn borrow_raw_str(s: &str) {}

let owned = DatabaseName::new(String::from("mongo"));
borrow_raw_str(owned.as_str());
take_raw_string(owned.take());
```

For more information, see the [documentation on docs.rs](https://docs.rs/aliri_braid).
