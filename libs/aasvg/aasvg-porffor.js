// aasvg-porffor.js - Porffor-compatible version of aasvg
// Exports: render(diagramString) -> svgString
//
// Simplified version avoiding object methods (Porffor limitation)

// Note: Porffor has issues with module-level const strings in concatenation
// So we use inline literals instead

function escapeHTMLEntities(str) {
    let result = "";
    for (let i = 0; i < str.length; i++) {
        const c = str.charAt(i);
        if (c === '&') result += '&amp;';
        else if (c === '<') result += '&lt;';
        else if (c === '>') result += '&gt;';
        else if (c === '"') result += '&quot;';
        else result += c;
    }
    return result;
}

function removeLeadingSpace(str) {
    const lineArray = str.split('\n');
    const len = lineArray.length;

    let minimum = 999999;
    for (let i = 0; i < len; i++) {
        const line = lineArray[i];
        if (line.trim() !== '') {
            let spaceCount = 0;
            for (let j = 0; j < line.length; j++) {
                const c = line.charAt(j);
                if (c === ' ' || c === '\t') spaceCount++;
                else break;
            }
            if (spaceCount < minimum) minimum = spaceCount;
        }
    }

    if (minimum === 0 || minimum === 999999) {
        return str;
    }

    let result = '';
    for (let i = 0; i < len; i++) {
        result += lineArray[i].substring(minimum) + '\n';
    }

    return result;
}

// Grid accessors as standalone functions
function gridGet(lines, x, y) {
    if (y < 0 || y >= lines.length) return ' ';
    const line = lines[y];
    if (x < 0 || x >= line.length) return ' ';
    return line.charAt(x);
}

function gridWidth(lines) {
    let w = 0;
    for (let i = 0; i < lines.length; i++) {
        if (lines[i].length > w) w = lines[i].length;
    }
    return w;
}

function gridHeight(lines) {
    let h = lines.length;
    if (h > 0 && lines[h - 1] === '') h--;
    return h;
}

// Character classification
function isVertex(c) { return c === '+' || c === '.' || c === "'" || c === '`' || c === ','; }
function isSolidHLine(c) { return c === '-' || c === '+'; }
function isSolidVLine(c) { return c === '|' || c === '+'; }
function isArrowHead(c) { return c === '>' || c === '<' || c === '^' || c === 'v' || c === 'V'; }
function isPoint(c) { return c === 'o' || c === '*'; }

export function render(diagramString) {
    const processed = removeLeadingSpace(diagramString);
    const lines = processed.split('\n');

    const width = gridWidth(lines);
    const height = gridHeight(lines);

    const SCALE = 8;
    const ASPECT = 2;

    const svgWidth = (width + 1) * SCALE;
    const svgHeight = (height + 1) * SCALE * ASPECT;

    let svg = '<svg xmlns="http://www.w3.org/2000/svg" version="1.1"';
    svg += ' width="' + svgWidth + '" height="' + svgHeight + '"';
    svg += ' viewBox="0 0 ' + svgWidth + ' ' + svgHeight + '"';
    svg += ' class="diagram" text-anchor="middle" font-family="monospace"';
    svg += ' font-size="13px" stroke-linecap="round">\n';

    // Track what we've drawn using a flat array
    const usedSize = height * width;
    const used = [];
    for (let i = 0; i < usedSize; i++) {
        used.push(false);
    }

    // Draw horizontal lines
    for (let y = 0; y < height; y++) {
        let x = 0;
        while (x < width) {
            const c = gridGet(lines, x, y);
            if (isSolidHLine(c)) {
                const startX = x;
                while (x < width && isSolidHLine(gridGet(lines, x, y))) {
                    used[y * width + x] = true;
                    x++;
                }
                if (x - startX >= 2) {
                    const x1 = (startX + 1) * SCALE;
                    const x2 = x * SCALE;
                    const cy = (y + 1) * SCALE * ASPECT;
                    svg += '<line x1="' + x1 + '" y1="' + cy + '" x2="' + x2 + '" y2="' + cy + '" fill="none" stroke="black"/>\n';
                }
            } else {
                x++;
            }
        }
    }

    // Draw vertical lines
    for (let x = 0; x < width; x++) {
        let y = 0;
        while (y < height) {
            const c = gridGet(lines, x, y);
            if (isSolidVLine(c)) {
                const startY = y;
                while (y < height && isSolidVLine(gridGet(lines, x, y))) {
                    used[y * width + x] = true;
                    y++;
                }
                if (y - startY >= 2) {
                    const cx = (x + 1) * SCALE;
                    const y1 = (startY + 1) * SCALE * ASPECT;
                    const y2 = y * SCALE * ASPECT;
                    svg += '<line x1="' + cx + '" y1="' + y1 + '" x2="' + cx + '" y2="' + y2 + '" fill="none" stroke="black"/>\n';
                }
            } else {
                y++;
            }
        }
    }

    // Draw arrow heads
    for (let y = 0; y < height; y++) {
        for (let x = 0; x < width; x++) {
            const c = gridGet(lines, x, y);
            if (isArrowHead(c)) {
                const cx = (x + 1) * SCALE;
                const cy = (y + 1) * SCALE * ASPECT;
                let angle = 0;
                if (c === '>') angle = 0;
                else if (c === 'v' || c === 'V') angle = 90;
                else if (c === '<') angle = 180;
                else if (c === '^') angle = 270;

                const tipX = cx + 8;
                const tipY = cy;
                const backX = cx - 4;
                const upY = cy - 3;
                const dnY = cy + 3;

                svg += '<polygon points="' + tipX + ',' + tipY + ' ' + backX + ',' + upY + ' ' + backX + ',' + dnY + '"';
                svg += ' fill="black" transform="rotate(' + angle + ',' + cx + ',' + cy + ')"/>\n';
                used[y * width + x] = true;
            }
        }
    }

    // Draw points (circles)
    for (let y = 0; y < height; y++) {
        for (let x = 0; x < width; x++) {
            const c = gridGet(lines, x, y);
            if (isPoint(c)) {
                const cx = (x + 1) * SCALE;
                const cy = (y + 1) * SCALE * ASPECT;
                if (c === '*') {
                    svg += '<circle cx="' + cx + '" cy="' + cy + '" r="6" fill="black"/>\n';
                } else {
                    svg += '<circle cx="' + cx + '" cy="' + cy + '" r="6" fill="white" stroke="black"/>\n';
                }
                used[y * width + x] = true;
            }
        }
    }

    // Draw remaining text
    svg += '<g class="text">\n';
    for (let y = 0; y < height; y++) {
        for (let x = 0; x < width; x++) {
            if (!used[y * width + x]) {
                const c = gridGet(lines, x, y);
                if (c !== ' ' && !isVertex(c)) {
                    const px = (x + 1) * SCALE;
                    const py = 4 + (y + 1) * SCALE * ASPECT;
                    svg += '<text x="' + px + '" y="' + py + '">' + escapeHTMLEntities(c) + '</text>\n';
                }
            }
        }
    }
    svg += '</g>\n';
    svg += '</svg>';

    return svg;
}
