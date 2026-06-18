#!/usr/bin/env node
const { diagramToSVG } = require("./markdeep-diagram.js");
const VERSION = "aasvg 0.4.0";

function usage() {
    console.warn("Turn ASCII art into SVG");
    console.warn();
    console.warn("Usage: aasvg [options] < <text> > <svg>");
    console.warn();
    console.warn("    --disable-text    Disable simple text");
    console.warn("    --grid            Draw a grid (debugging)");
    console.warn("    --spaces=<n>      Split text after <n> spaces [default: 2]");
    console.warn("                      (0 means place every character separately)");
    console.warn("    --stretch         Stretch text to better fit it")
    console.warn("                      (use with --spaces > 0; uses advanced SVG)");
    console.warn("    --fill            Omit width and height attributes");
    console.warn("    --width=<n>       Set the viewbox width to <n> characters");
    console.warn("    --height=<n>      Set the viewbox height to <n> characters");
    console.warn("    --backdrop        Draw a backdrop");
    console.warn("    --source          Draw an overlay with source text");
    console.warn("    --<attr>=<value>  Set SVG attribute <attr> to <value>");
    console.warn("    --version         Show the version and exit");
    process.exit(2);
}

async function read() {
    let input = "";
    process.stdin.setEncoding("utf8");
    process.stdin.on("readable", () => {
        let chunk;
        // Use a loop to make sure we read all available data.
        while ((chunk = process.stdin.read()) !== null) {
            input += chunk.replace(/\r/g, "");
        }
    });
    return await new Promise((resolve, reject) => {
        process.stdin.on("end", () => {
            resolve(input);
        });
        process.stdin.on("error", e => {
            reject(e);
        });
    });
}

function i(o, a) {
    let v = parseInt(a.substring(o.length + 3), 10);
    if (isNaN(v)) {
        console.warn(`Invalid value for --${o} option`);
        process.exit(2);
    }
    return v;
}

(async function main() {
    let options = { style: {} };
    process.argv.slice(2).forEach(a => {
        if (a === "--disable-text") {
            options.disableText = true;
        } else if (a === "--grid") {
            options.grid = true;
        } else if (a === "--stretch") {
            options.stretch = true;
        } else if (a === "--backdrop") {
            options.backdrop = true;
        } else if (a === "--source") {
            options.source = true;
        } else if (a === "--fill") {
            options.fill = true;
        } else if (a.startsWith("--width=")) {
            options.width = i("width", a);
        } else if (a.startsWith("--height=")) {
            options.height = i("height", a);
        } else if (a.startsWith("--spaces=")) {
            options.spaces = i("spaces", a);
        } else if (a === "--version") {
            console.log(VERSION);
            process.exit();
        } else {
            let s = a.substring(2).split("=");
            if (a.substring(0, 2) === "--" && s.length === 2) {
                options.style[s[0]] = s[1];
            } else {
                usage();
            }
        }
    })
    const txt = await read();
    const svg = diagramToSVG(txt, options);
    console.log(svg);
})();
