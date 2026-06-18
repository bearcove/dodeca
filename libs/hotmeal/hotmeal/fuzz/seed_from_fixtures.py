#!/usr/bin/env python3
"""Generate fuzzer corpus seeds from HTML fixtures.

Creates pairs of HTML snippets (separated by 0xFF) for the apply_parity fuzzer.
"""

import os
import hashlib
from pathlib import Path

FIXTURES_DIR = Path(__file__).parent.parent / "tests" / "fixtures"
CORPUS_DIR = Path(__file__).parent / "corpus" / "apply_parity"

def extract_snippets(html: str, max_len: int = 2000) -> list[str]:
    """Extract interesting HTML snippets from a document."""
    snippets = []
    
    # Find balanced tag regions
    import re
    
    # Match common container elements with their content
    patterns = [
        r'<div[^>]*>.*?</div>',
        r'<p[^>]*>.*?</p>',
        r'<ul[^>]*>.*?</ul>',
        r'<ol[^>]*>.*?</ol>',
        r'<li[^>]*>.*?</li>',
        r'<span[^>]*>.*?</span>',
        r'<a[^>]*>.*?</a>',
        r'<table[^>]*>.*?</table>',
        r'<tr[^>]*>.*?</tr>',
        r'<td[^>]*>.*?</td>',
        r'<th[^>]*>.*?</th>',
        r'<section[^>]*>.*?</section>',
        r'<article[^>]*>.*?</article>',
        r'<nav[^>]*>.*?</nav>',
        r'<header[^>]*>.*?</header>',
        r'<footer[^>]*>.*?</footer>',
        r'<h[1-6][^>]*>.*?</h[1-6]>',
        r'<form[^>]*>.*?</form>',
        r'<input[^>]*>',
        r'<button[^>]*>.*?</button>',
    ]
    
    for pattern in patterns:
        for match in re.finditer(pattern, html, re.DOTALL | re.IGNORECASE):
            snippet = match.group(0)
            if len(snippet) <= max_len and len(snippet) > 10:
                snippets.append(snippet)
    
    return snippets

def make_seed(html_a: str, html_b: str) -> bytes:
    """Create a seed file: html_a + 0xFF + html_b"""
    return html_a.encode('utf-8') + b'\xff' + html_b.encode('utf-8')

def seed_hash(data: bytes) -> str:
    return hashlib.sha1(data).hexdigest()

def main():
    CORPUS_DIR.mkdir(parents=True, exist_ok=True)
    
    all_snippets = []
    
    # Collect snippets from all fixtures
    for html_file in FIXTURES_DIR.glob("*.html"):
        print(f"Processing {html_file.name}...")
        try:
            html = html_file.read_text(encoding='utf-8', errors='ignore')
            snippets = extract_snippets(html)
            print(f"  Found {len(snippets)} snippets")
            all_snippets.extend(snippets)
        except Exception as e:
            print(f"  Error: {e}")
    
    print(f"\nTotal snippets: {len(all_snippets)}")
    
    # Deduplicate
    all_snippets = list(set(all_snippets))
    print(f"Unique snippets: {len(all_snippets)}")
    
    # Create seeds by pairing snippets
    seeds_created = 0
    
    # Pair each snippet with a modified version of itself
    for snippet in all_snippets[:500]:  # Limit to avoid too many
        # Seed: original vs slightly modified
        modified = snippet.replace(">", "> ").replace("<", " <")  # Add spaces
        if modified != snippet:
            seed = make_seed(snippet, modified)
            seed_file = CORPUS_DIR / f"fixture_mod_{seed_hash(seed)}"
            if not seed_file.exists():
                seed_file.write_bytes(seed)
                seeds_created += 1
    
    # Pair different snippets together
    import random
    random.seed(42)
    pairs = min(500, len(all_snippets) * 2)
    for _ in range(pairs):
        a = random.choice(all_snippets)
        b = random.choice(all_snippets)
        if a != b:
            seed = make_seed(a, b)
            seed_file = CORPUS_DIR / f"fixture_pair_{seed_hash(seed)}"
            if not seed_file.exists():
                seed_file.write_bytes(seed)
                seeds_created += 1
    
    # Add some hand-crafted realistic transitions
    realistic_pairs = [
        ("<p>Hello</p>", "<p>Hello World</p>"),
        ("<ul><li>A</li></ul>", "<ul><li>A</li><li>B</li></ul>"),
        ("<div class='old'>text</div>", "<div class='new'>text</div>"),
        ("<span>one</span>", "<span>two</span>"),
        ("<a href='#'>link</a>", "<a href='/page'>link</a>"),
        ("<input type='text'>", "<input type='password'>"),
        ("<div><p>nested</p></div>", "<div><span>changed</span></div>"),
        ("<table><tr><td>1</td></tr></table>", "<table><tr><td>2</td></tr></table>"),
    ]
    
    for a, b in realistic_pairs:
        seed = make_seed(a, b)
        seed_file = CORPUS_DIR / f"fixture_hand_{seed_hash(seed)}"
        if not seed_file.exists():
            seed_file.write_bytes(seed)
            seeds_created += 1
    
    print(f"\nCreated {seeds_created} new seed files in {CORPUS_DIR}")

if __name__ == "__main__":
    main()
