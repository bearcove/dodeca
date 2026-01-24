# Fuzzing targets for hotmeal

# Run the diff/apply roundtrip fuzzer with HTML dictionary
fuzz-mutate *ARGS:
    cd hotmeal/fuzz && cargo +nightly fuzz run mutate -- -dict=html.dict {{ARGS}}

# Minimize a crash artifact
tmin-mutate ARTIFACT:
    cd hotmeal/fuzz && cargo +nightly fuzz tmin mutate {{ARTIFACT}}

# Run with coverage report
fuzz-mutate-cov *ARGS:
    cd hotmeal/fuzz && cargo +nightly fuzz coverage mutate -- -dict=html.dict {{ARGS}}

# List all fuzz targets
list:
    cd hotmeal/fuzz && cargo +nightly fuzz list
