# Find Crate Version

To find the latest version of a Rust crate, use the crates.io API:

```
https://crates.io/api/v1/crates/{crate_name}
```

Use WebFetch with this URL and ask for the latest version number.

Example prompt: "What is the latest version number of this crate?"

The response will include `default_version` and `newest_version` fields with the current version.

---

Now find the latest version of: $ARGUMENTS
