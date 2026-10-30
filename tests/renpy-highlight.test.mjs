// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { highlightRenpyLine } from '../src/lib/renpy-highlight.ts';

test('keywords keep valid markup', () => {
  assert.equal(
    highlightRenpyLine('    show eileen happy'),
    '    <span class="tok-keyword">show</span> eileen happy'
  );
  assert.equal(
    highlightRenpyLine('    menu:'),
    '    <span class="tok-keyword">menu:</span>'
  );
});

test('words inside strings are not keywords', () => {
  assert.equal(
    highlightRenpyLine('    e "come with me"'),
    '    e <span class="tok-string">"come with me"</span>'
  );
});

test('escaped quotes and apostrophes stay inside one string', () => {
  assert.equal(
    highlightRenpyLine(String.raw`    e "don't say \"if\""`),
    String.raw`    e <span class="tok-string">"don't say \"if\""</span>`
  );
});

test('an unterminated quote is left as text', () => {
  assert.equal(highlightRenpyLine("    it's show"), '    it\'s <span class="tok-keyword">show</span>');
});

test('file text cannot produce markup', () => {
  assert.equal(
    highlightRenpyLine('    e "<img src=x onerror=alert(1)>" <b>'),
    '    e <span class="tok-string">"&lt;img src=x onerror=alert(1)&gt;"</span> &lt;b&gt;'
  );
  assert.equal(highlightRenpyLine('# <b>'), '<span class="tok-comment"># &lt;b&gt;</span>');
});

test('blank lines keep their height', () => {
  assert.equal(highlightRenpyLine('   '), '&nbsp;');
});
