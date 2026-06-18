#!/usr/bin/env node
/**
 * Conformance test runner - compares JS and Rust aasvg output
 *
 * Usage: node tests/conformance/run.js
 *
 * Both versions run with --stretch option.
 */

const { diagramToSVG } = require("../../markdeep-diagram.js");
const { execSync, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const FIXTURES_DIR = path.join(__dirname);
const PROJECT_ROOT = path.join(__dirname, "../..");

// Parse SVG and extract structural elements
function parseSvgStructure(svg) {
    const structure = {
        paths: [],
        pathCommands: 0, // Count of individual path commands (M...L or C etc.)
        polygons: [],
        circles: [],
        texts: [],
        rects: [],
        viewBox: null,
        width: null,
        height: null,
    };

    // Extract dimensions
    const widthMatch = svg.match(/width="(\d+)"/);
    const heightMatch = svg.match(/height="(\d+)"/);
    if (widthMatch) structure.width = parseInt(widthMatch[1], 10);
    if (heightMatch) structure.height = parseInt(heightMatch[1], 10);

    // Extract viewBox
    const viewBoxMatch = svg.match(/viewBox="([^"]+)"/);
    if (viewBoxMatch) {
        structure.viewBox = viewBoxMatch[1];
    }

    // Extract paths (d attribute contains the path data)
    const pathRegex = /<path[^>]*d="([^"]+)"[^>]*>/g;
    let match;
    while ((match = pathRegex.exec(svg)) !== null) {
        const pathData = normalizePath(match[1]);
        structure.paths.push(pathData);
        // Count M commands to get number of sub-paths
        structure.pathCommands += (pathData.match(/M /g) || []).length;
    }

    // Extract polygons (points attribute)
    const polygonRegex = /<polygon[^>]*points="([^"]+)"[^>]*>/g;
    while ((match = polygonRegex.exec(svg)) !== null) {
        structure.polygons.push(normalizePoints(match[1]));
    }

    // Extract circles (cx, cy, r)
    const circleRegex = /<circle[^>]*cx="([^"]+)"[^>]*cy="([^"]+)"[^>]*r="([^"]+)"[^>]*>/g;
    while ((match = circleRegex.exec(svg)) !== null) {
        structure.circles.push({
            cx: roundNum(match[1]),
            cy: roundNum(match[2]),
            r: roundNum(match[3])
        });
    }

    // Also try alternate circle attribute order
    const circleRegex2 = /<circle[^>]*r="([^"]+)"[^>]*cx="([^"]+)"[^>]*cy="([^"]+)"[^>]*>/g;
    while ((match = circleRegex2.exec(svg)) !== null) {
        structure.circles.push({
            cx: roundNum(match[2]),
            cy: roundNum(match[3]),
            r: roundNum(match[1])
        });
    }

    // Extract text elements
    const textRegex = /<text[^>]*>([^<]*)<\/text>/g;
    while ((match = textRegex.exec(svg)) !== null) {
        structure.texts.push(match[1]);
    }

    // Extract rects
    const rectRegex = /<rect[^>]*/g;
    while ((match = rectRegex.exec(svg)) !== null) {
        structure.rects.push(match[0]);
    }

    return structure;
}

// Round a number string to 1 decimal place
function roundNum(s) {
    return Math.round(parseFloat(s) * 10) / 10;
}

// Normalize path data for comparison (round numbers, normalize whitespace)
function normalizePath(d) {
    return d
        .replace(/([0-9]+\.[0-9])[0-9]+/g, '$1') // Truncate to 1 decimal place
        .replace(/\s+/g, ' ')
        .trim();
}

// Normalize polygon points
function normalizePoints(pts) {
    return pts
        .replace(/([0-9]+\.[0-9])[0-9]+/g, '$1')
        .replace(/\s+/g, ' ')
        .trim();
}

// Run Rust version
function runRust(input) {
    const result = spawnSync("cargo", ["run", "--example", "conformance_runner", "--release"], {
        cwd: PROJECT_ROOT,
        input: input,
        encoding: 'utf8',
        maxBuffer: 10 * 1024 * 1024,
        stdio: ['pipe', 'pipe', 'pipe']
    });

    if (result.error) {
        throw result.error;
    }

    if (result.status !== 0) {
        throw new Error(`Rust build/run failed: ${result.stderr}`);
    }

    return result.stdout;
}

// Compare two structures
function compareStructures(jsStruct, rustStruct, fixtureName) {
    const issues = [];

    // Compare dimensions
    if (jsStruct.width !== rustStruct.width) {
        issues.push(`Width: JS=${jsStruct.width} vs Rust=${rustStruct.width}`);
    }
    if (jsStruct.height !== rustStruct.height) {
        issues.push(`Height: JS=${jsStruct.height} vs Rust=${rustStruct.height}`);
    }

    // Compare path command counts (accounts for multi-command paths)
    if (jsStruct.pathCommands !== rustStruct.pathCommands) {
        issues.push(`Path commands: JS=${jsStruct.pathCommands} vs Rust=${rustStruct.pathCommands}`);
    }

    // Compare polygon counts (arrows)
    if (jsStruct.polygons.length !== rustStruct.polygons.length) {
        issues.push(`Polygon count: JS=${jsStruct.polygons.length} vs Rust=${rustStruct.polygons.length}`);
    }

    // Compare circle counts (points)
    if (jsStruct.circles.length !== rustStruct.circles.length) {
        issues.push(`Circle count: JS=${jsStruct.circles.length} vs Rust=${rustStruct.circles.length}`);
    }

    // Compare text counts
    if (jsStruct.texts.length !== rustStruct.texts.length) {
        issues.push(`Text count: JS=${jsStruct.texts.length} vs Rust=${rustStruct.texts.length}`);
    }

    // Compare text content (sorted for order-independence)
    const jsTexts = jsStruct.texts.slice().sort();
    const rustTexts = rustStruct.texts.slice().sort();
    for (let i = 0; i < Math.min(jsTexts.length, rustTexts.length); i++) {
        if (jsTexts[i] !== rustTexts[i]) {
            issues.push(`Text mismatch: JS="${jsTexts[i]}" vs Rust="${rustTexts[i]}"`);
        }
    }

    return issues;
}

// Main test runner
async function main() {
    // Build Rust first
    console.log("Building Rust crate...");
    try {
        execSync("cargo build --release --example conformance_runner", {
            cwd: PROJECT_ROOT,
            stdio: 'pipe'
        });
    } catch (e) {
        console.error("Failed to build Rust crate:", e.message);
        process.exit(1);
    }

    const fixtures = fs.readdirSync(FIXTURES_DIR)
        .filter(f => f.endsWith('.txt'))
        .sort();

    console.log(`\nRunning conformance tests on ${fixtures.length} fixtures...\n`);

    let passed = 0;
    let failed = 0;
    const failures = [];

    for (const fixture of fixtures) {
        const fixturePath = path.join(FIXTURES_DIR, fixture);
        const input = fs.readFileSync(fixturePath, 'utf8');
        const name = fixture.replace('.txt', '');

        try {
            // Run JS version with stretch
            const jsOptions = { stretch: true, spaces: 2, style: {} };
            const jsSvg = diagramToSVG(input, jsOptions);
            const jsStruct = parseSvgStructure(jsSvg);

            // Run Rust version
            const rustSvg = runRust(input);
            const rustStruct = parseSvgStructure(rustSvg);

            // Compare
            const issues = compareStructures(jsStruct, rustStruct, name);

            if (issues.length === 0) {
                console.log(`✓ ${name}`);
                console.log(`  Paths: ${jsStruct.pathCommands}, Arrows: ${jsStruct.polygons.length}, Points: ${jsStruct.circles.length}, Text: ${jsStruct.texts.length}`);
                passed++;
            } else {
                console.log(`✗ ${name}`);
                for (const issue of issues) {
                    console.log(`  - ${issue}`);
                }
                failures.push({ name, issues });
                failed++;

                // Save both outputs for debugging
                const jsOutPath = path.join(FIXTURES_DIR, `${name}.js.svg`);
                const rustOutPath = path.join(FIXTURES_DIR, `${name}.rust.svg`);
                fs.writeFileSync(jsOutPath, jsSvg);
                fs.writeFileSync(rustOutPath, rustSvg);
                console.log(`  Saved: ${name}.js.svg, ${name}.rust.svg`);
            }
        } catch (e) {
            console.log(`✗ ${name}: ${e.message}`);
            failures.push({ name, error: e.message });
            failed++;
        }
    }

    console.log(`\n${'='.repeat(60)}`);
    console.log(`Results: ${passed} passed, ${failed} failed`);

    if (failures.length > 0) {
        console.log('\nSummary of failures:');
        for (const f of failures) {
            if (f.error) {
                console.log(`  - ${f.name}: ${f.error}`);
            } else {
                console.log(`  - ${f.name}: ${f.issues.length} issue(s)`);
            }
        }
        process.exit(1);
    }
}

main().catch(e => {
    console.error(e);
    process.exit(1);
});
