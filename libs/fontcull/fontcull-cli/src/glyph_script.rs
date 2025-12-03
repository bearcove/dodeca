/// JavaScript code that runs in the browser to extract glyphs per font-family
/// This is based on glyphhanger's glyphhanger-script.js
pub const GLYPH_SCRIPT: &str = r#"
(() => {
    const fontFamilySets = {};

    function saveGlyph(char, fontFamily) {
        const code = char.charCodeAt(0);
        if (code === 0 || isNaN(code)) return;

        // Add to specific family set
        const familyKey = fontFamily || '*';
        if (!fontFamilySets[familyKey]) {
            fontFamilySets[familyKey] = [];
        }
        if (!fontFamilySets[familyKey].includes(code)) {
            fontFamilySets[familyKey].push(code);
        }

        // Also add to universal set
        if (!fontFamilySets['*']) {
            fontFamilySets['*'] = [];
        }
        if (!fontFamilySets['*'].includes(code)) {
            fontFamilySets['*'].push(code);
        }
    }

    function saveGlyphs(text, fontFamily) {
        for (const char of text) {
            saveGlyph(char, fontFamily);
        }
    }

    function getFontFamily(node, pseudo) {
        try {
            const style = window.getComputedStyle(node, pseudo || null);
            let family = style.getPropertyValue('font-family');
            // Take first font in the stack
            if (family) {
                family = family.split(',')[0].trim().replace(/['"]/g, '');
            }
            return family || '*';
        } catch (e) {
            return '*';
        }
    }

    function getTextTransform(node) {
        try {
            return window.getComputedStyle(node).getPropertyValue('text-transform');
        } catch (e) {
            return 'none';
        }
    }

    function getFontVariant(node) {
        try {
            return window.getComputedStyle(node).getPropertyValue('font-variant');
        } catch (e) {
            return 'normal';
        }
    }

    function processText(text, node, fontFamily) {
        const transform = getTextTransform(node);
        const variant = getFontVariant(node);

        // Handle text-transform
        if (transform === 'uppercase') {
            text = text.toUpperCase();
        } else if (transform === 'lowercase') {
            text = text.toLowerCase();
        } else if (transform === 'capitalize') {
            // Include both cases for capitalize
            saveGlyphs(text.toLowerCase(), fontFamily);
            saveGlyphs(text.toUpperCase(), fontFamily);
            return;
        }

        // Handle small-caps: include both upper and lower
        if (variant && variant.includes('small-caps')) {
            saveGlyphs(text.toLowerCase(), fontFamily);
            saveGlyphs(text.toUpperCase(), fontFamily);
            return;
        }

        saveGlyphs(text, fontFamily);
    }

    function getPseudoContent(node, pseudo) {
        try {
            const style = window.getComputedStyle(node, pseudo);
            let content = style.getPropertyValue('content');
            if (content && content !== 'none' && content !== 'normal') {
                // Remove quotes
                content = content.replace(/^["']|["']$/g, '');
                // Handle attr() - just skip for now
                if (content.startsWith('attr(')) {
                    return '';
                }
                return content;
            }
        } catch (e) {}
        return '';
    }

    function getTextNodes(node) {
        const textNodes = [];
        const walker = document.createTreeWalker(
            node,
            NodeFilter.SHOW_TEXT,
            null,
            false
        );
        let textNode;
        while (textNode = walker.nextNode()) {
            textNodes.push(textNode);
        }
        return textNodes;
    }

    // Process all elements
    const allElements = document.querySelectorAll('*');

    for (const node of allElements) {
        // Skip script and style tags
        const tagName = node.tagName.toLowerCase();
        if (tagName === 'script' || tagName === 'style' || tagName === 'noscript') {
            continue;
        }

        // Process text nodes
        const textNodes = getTextNodes(node);
        for (const textNode of textNodes) {
            const text = textNode.nodeValue;
            if (text && text.trim()) {
                const fontFamily = getFontFamily(textNode.parentElement);
                processText(text, textNode.parentElement, fontFamily);
            }
        }

        // Process ::before pseudo-element
        const beforeContent = getPseudoContent(node, ':before');
        if (beforeContent) {
            const fontFamily = getFontFamily(node, ':before');
            processText(beforeContent, node, fontFamily);
        }

        // Process ::after pseudo-element
        const afterContent = getPseudoContent(node, ':after');
        if (afterContent) {
            const fontFamily = getFontFamily(node, ':after');
            processText(afterContent, node, fontFamily);
        }
    }

    return fontFamilySets;
})()
"#;
