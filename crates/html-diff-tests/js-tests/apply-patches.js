// Apply DOM patches using jsdom - validates that our patches work in a real DOM
const { JSDOM } = require('jsdom');

/**
 * Apply a list of patches to an HTML document
 * @param {string} html - The original HTML
 * @param {Array} patches - Array of patch objects
 * @returns {string} - The resulting HTML after patches
 */
function applyPatches(html, patches) {
    const dom = new JSDOM(html);
    const document = dom.window.document;
    
    for (const patch of patches) {
        applyPatch(document, patch);
    }
    
    // Return the body's innerHTML (what we care about for comparison)
    return document.body.innerHTML;
}

function applyPatch(document, patch) {
    const type = patch.type;
    const path = patch.path || [];
    
    switch (type) {
        case 'SetText': {
            const node = findNode(document, path);
            node.textContent = patch.text;
            break;
        }
        case 'SetAttribute': {
            const el = findElement(document, path);
            el.setAttribute(patch.name, patch.value);
            break;
        }
        case 'RemoveAttribute': {
            const el = findElement(document, path);
            el.removeAttribute(patch.name);
            break;
        }
        case 'Remove': {
            const node = findNode(document, path);
            node.parentNode.removeChild(node);
            break;
        }
        case 'Replace': {
            const el = findElement(document, path);
            el.outerHTML = patch.html;
            break;
        }
        case 'InsertBefore': {
            const el = findElement(document, path);
            el.insertAdjacentHTML('beforebegin', patch.html);
            break;
        }
        case 'InsertAfter': {
            const el = findElement(document, path);
            el.insertAdjacentHTML('afterend', patch.html);
            break;
        }
        case 'AppendChild': {
            const el = findElement(document, path);
            el.insertAdjacentHTML('beforeend', patch.html);
            break;
        }
        default:
            throw new Error(`Unknown patch type: ${type}`);
    }
}

function findNode(document, path) {
    let current = document.body;
    
    for (const idx of path) {
        const children = current.childNodes;
        if (idx >= children.length) {
            throw new Error(`Child index ${idx} out of bounds (${children.length} children)`);
        }
        current = children[idx];
    }
    
    return current;
}

function findElement(document, path) {
    const node = findNode(document, path);
    if (node.nodeType !== 1) { // ELEMENT_NODE
        throw new Error(`Node at path [${path.join(', ')}] is not an element (type: ${node.nodeType})`);
    }
    return node;
}

// Read from stdin and process
let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', chunk => input += chunk);
process.stdin.on('end', () => {
    try {
        const { html, patches } = JSON.parse(input);
        const result = applyPatches(html, patches);
        console.log(JSON.stringify({ success: true, html: result }));
    } catch (e) {
        console.log(JSON.stringify({ success: false, error: e.message }));
        process.exit(1);
    }
});
