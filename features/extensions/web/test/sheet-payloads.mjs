// The payloads of the second security review of architect#601 (its
// `gen.mjs`, kept here so this boundary keeps its judge), and the URL
// traversal family that escaped it. `P` is the view's own file prefix.
export const P = '/_ext/island/demo.abc/';
const E = 'https://e.vil/x';
const esc = {
  'host \\68 sp': ':\\68 ost{position:fixed}', 'host \\68': ':\\68ost{color:red}', 'host \\000068': ':\\000068ost{color:red}',
  'host \\48 upper': ':\\48 OST{position:absolute;left:-90px;top:-90px;width:400px;height:400px;background:red}', 'host esc tab': ':\\68\tost{color:red}',
  'host esc nl': ':\\68\nost{color:red}', 'host esc crlf': ':\\68\r\nost{color:red}', 'escaped colon': '\\:host{color:red}', 'esc colon2': '.a\\:host{color:red}',
  'string escaped nl': '.x::before{content:"a\\\nb";background:red}', 'backslash end': '.x{color:red}\\', 'backslash end in str': '.x{content:"\\',
  'url escaped ident': `.x{background:\\75 rl(${E})}`, 'url esc in string': `.x{background:url("\\68ttps://e.vil/x")}`, 'url unquoted esc': `.x{background:url(\\68ttps://e.vil/x)}`,
  'url fn upper': `.x{background:URL(${E})}`, 'u-comment-rl': `.x{background:u/**/rl(${E})}`, 'url/**/(': `.x{background:url/**/("${E}")}`,
};
const comments = {
  'ho/**/st': ':ho/**/st{color:red}', 'value comment': '.x{background:red/**/;color:red}', 'unterminated comment': '.x{color:red}/*', 'comment hides brace': '.x{color:red/*}*/}',
  'comment in at': '@me/**/dia all{.x{color:red}}', 'comment string trick': `.x{content:"/*"}:host{position:fixed}.y{content:"*/"}`,
};
const eof = {
  'a{b:c': '.x{color:red', 'media unterminated url': `@media all{.x{background:url(${E}`, 'unterminated string': '.x{content:"abc', 'unterminated fn': '.x{width:calc(1px',
  'unterminated at': '@media all', 'unterminated at;': '@import', 'stray }': '.x{color:red}}:host{position:fixed}', 'stray ) in block': '.x{color:red)}', 'bad url then rule': `.x{background:url(a b)}:host{position:fixed}`,
};
const markup = { '</style>': '.x{content:"</style><img src=https://e.vil/x>"}', '</style> bare': '.x{color:red}</style><img src=x>', 'CDO': '<!-- .x{color:red} -->', 'CDC only': '.x{color:red}-->', 'cdo in str': '.x{content:"<!--"}' };
const sel = {
  '&': '&{position:absolute;left:-100px;top:-100px;width:600px;height:600px;background:red}', ':is(&)': ':is(&){position:absolute;left:-100px;top:-100px;width:600px;height:600px;background:red}',
  'nested & in .card': '.card{&{position:absolute;left:-100px;top:-100px;width:600px;height:600px;background:red}}', ':is(:host)': ':is(:host){color:red}', ':where(:host)': ':where(:host){color:red}', ':not(:host)': ':not(:host){color:red}',
  ':has(:host)': ':has(.x){color:red}', ':host()': ':host(.a){color:red}', ':host-context()': ':host-context(body){color:red}', '::part': '::part(x){color:red}', '::slotted': '::slotted(*){color:red}',
  ':root': ':root{position:fixed}', ':scope': ':scope{color:red}', 'html': 'html{color:red}', 'body': 'body{color:red}', 'HTML upper': 'HTML{color:red}',
  '* abs': '*{position:absolute;left:-100px;top:-100px;width:600px;height:600px;background:red}', '.architect-view': '.architect-view{color:red}', 'esc .architect-view': '.architect\\-view{background:red;width:9999px}',
  'attr class': '[class~=architect-view]{position:absolute;left:-100px;top:-100px;width:600px;height:600px;background:red}', 'attr class^': '[class^=arch]{margin:-100px;width:600px;height:600px;background:red}',
  'div:first-child': 'div:first-child{margin:-100px;width:600px;height:600px;background:red}', 'div div': 'div{transform:scale(5);background:red}',
  'theme w/ :host decl': ':root,:host{--x:1;position:fixed}', 'theme vars only': ':root,:host{--x:red}.card{background:var(--x)}', '::before abs': '.card::before{content:"";position:absolute;inset:-500px;background:red}',
  ':host esc in :is': ':is(:\\68 ost){color:red}', '::backdrop': '.card::backdrop{background:red}', ':popover-open': '.card:popover-open{color:red}', ':modal': '.card:modal{color:red}',
  'webkit pseudo fn': '.x:-moz-any(a){color:red}', 'attr nested fn': '[a=b i]{color:red}', '::view-transition': '::view-transition-old(root){background:red}', ':state': ':state(x){color:red}',
};
const at = {};
for (const a of ['@import "x.css";', `@import url(${E});`, `@import url("${P}x.css");`, `@import "${P}x.css" layer(a);`, '@namespace svg url(http://www.w3.org/2000/svg);', '@charset "utf-8";',
  `@font-face{font-family:x;src:url(${P}f.woff)}`, `@font-face{font-family:x;src:local(Arial);unicode-range:U+0-10FFFF}`, `@property --u{syntax:"<url>";inherits:true;initial-value:url(${E})}`,
  `@property --u{syntax:"<url>";inherits:true;initial-value:url(${P}a.png)}.card{background:var(--u)}`, `@property --u{syntax:"*";inherits:true;initial-value:url(${E})}`,
  '@layer a{.card{position:absolute;inset:-500px;background:red}}', '@layer a,b;', '@container (min-width:1px){.card{margin:-200px;background:red;width:900px}}', '@scope (.card){:scope{color:red}}',
  '@starting-style{.card{color:red}}', '@view-transition{navigation:auto}', '@page{margin:0}', '@counter-style x{system:cyclic;symbols:"a"}', '@keyframes k{to{position:fixed;inset:0;background:red}}.card{animation:k 1s infinite}',
  '@keyframes k{from{transform:translate(-500px,-500px) scale(10)}to{transform:translate(-500px,-500px) scale(10)}}.card{background:red;animation:k 100s infinite}',
  '@media all{:host{position:fixed}}', '@supports (display:grid){.card{position:absolute;inset:-500px;background:red}}', `@supports (background:url(${E})){.x{color:red}}`, '@media (x:attr(y)){.x{color:red}}',
  '@-moz-document url-prefix(){.x{color:red}}', '@MEDIA all{.x{color:red}}', '@\\6d edia all{.card{color:red}}', '@media all{@import "x";}', '@position-try --x{top:0}', '@font-feature-values x{}', '@font-palette-values --x{}', '@layer{.card{color:red}}', '@media all{@layer x{@media all{.card{color:green}}}}',
  '.card{@media all{position:absolute;inset:-500px;background:red}}', '.card{@layer{background:red;margin:-300px;width:900px;height:900px}}', '.card{@property --x{syntax:"*"}}']) at[a.slice(0, 70)] = a;
const vals = {};
const urlForms = [`url(${E})`, `url("${E}")`, `url( ${E} )`, `url(//e.vil/x)`, `image("${E}")`, `image-set("${E}" 1x)`, `-webkit-image-set("${E}" 1x)`, `image-set(url(${E}) 1x)`, `cross-fade(url(${E}),red)`, `element(#x)`, `paint(x)`, `src("${E}")`, `attr(data-secret url)`, `attr(data-secret type(<url>))`, `env(x, url(${E}))`, `var(--a, url(${E}))`, `url(${P}../../../x)`, `url(${P}%2e%2e/%2e%2e/x)`, `url("${P}.%2E/.%2e/.%2e/x")`, `url(${P}x?y)`, `url("${P}x#y")`, `url(${P}x%3F)`, `url(${P}\\2e\\2e/\\2e\\2e/x)`, `url("${P}x\\"")`, `url(${P})`, `url(${P}a.png)`];
for (const u of urlForms) for (const prop of ['background-image', 'cursor', 'list-style', 'mask', 'filter', 'clip-path', 'shape-outside', 'border-image', 'content']) vals[`${prop}: ${u}`] = `.card,.card::before{${prop}:${u}}`;
Object.assign(vals, {
  'var assemble url': `.card{--a:url(;--b:${E});background:var(--a)var(--b))}`, 'var legal url': `.card{--u:url(${E});background:var(--u)}`, 'var string': `.card{--u:"${E}";background:url(var(--u))}`,
  'custom prop braces': '.card{--x:{a:b};color:red}', 'custom prop url P': `.card{--u:url(${P}a.png);background:var(--u)}`, 'custom prop esc url': `.card{--u:\\75rl(${E});background:var(--u)}`,
  'behavior': '.card{behavior:url(x.htc)}', '-moz-binding': '.card{-moz-binding:url(x)}', 'expression': '.card{width:expression(alert(1))}', 'attr content': '.card::before{content:attr(data-secret)}',
  'counter': '.card::before{content:counter(x)}', 'image-rendering fn ok?': '.card{background:linear-gradient(red,blue)}', 'gradient with url': `.card{background:linear-gradient(red,blue),url(${E})}`,
});
const layout = {
  fixed: '.card{position:fixed;inset:0;background:red}', 'fixed upper': '.card{position:FIXED;inset:0;background:red}', 'fixed esc': '.card{position:\\66ixed;inset:0;background:red}', sticky: '.card{position:sticky;top:0}',
  'abs huge': '.card{position:absolute;left:-5000px;top:-5000px;width:99999px;height:99999px;background:red}', 'neg margin': '.card{margin:-500px;width:2000px;height:2000px;background:red}',
  transform: '.card{transform:translate(-300px,-300px) scale(20);background:red}', translate: '.card{translate:-300px -300px;scale:20;background:red}', 'z-index': '.card{position:relative;z-index:2147483647;top:-200px;background:red;height:900px}',
  'overflow visible !imp': '.card{overflow:visible!important}', 'contain none': '.card{contain:none;position:absolute;inset:-500px;background:red}', 'all unset': '*{all:unset}.card{position:absolute;inset:-500px;background:red}', 'all revert !imp': '*{all:revert!important}',
  'display contents': '.card{display:contents}', 'popover top layer': '.card{position:absolute;inset:-500px;background:red}', 'anchor': '.card{anchor-name:--a;position:absolute;top:anchor(--a top);background:red}',
  'view-transition-name': '.card{view-transition-name:x}', 'box-shadow': '.card{box-shadow:0 0 0 500px red}', outline: '.card{outline:500px solid red}', 'drop-shadow': '.card{filter:drop-shadow(0 0 0 red) drop-shadow(300px 300px 0 red);background:red}',
  'mix-blend': '.card{mix-blend-mode:difference;background:white;position:absolute;inset:-500px}', 'backdrop-filter': '.card{backdrop-filter:blur(10px)}', 'pointer-events': '.card{pointer-events:none}', 'cursor none': '.card{cursor:none}',
  'zoom': '.card{zoom:50;background:red}', 'width huge': '.card{width:100000px;height:100000px;background:red}', 'outline-offset': '.card{outline:2px solid red;outline-offset:300px}', 'border-image-outset': '.card{border:1px solid;border-image:linear-gradient(red,red) 1;border-image-outset:300px}',
  'text-shadow': '.card{text-shadow:300px 300px 0 red}', 'scroll-margin': '.card{scroll-margin:9999px}', 'margin !important': '.card{margin:-500px!important;background:red;width:2000px;height:2000px}', 'margin-inline !imp': '.card{margin-inline:-500px!important;margin-block:-500px!important;padding:900px!important;background:red!important}',
  'inset-inline !imp': '.card{position:absolute!important}', 'position abs !imp': '.card{position:absolute!important;inset-block-start:-500px!important;left:-500px!important;padding:900px;background:red}',
  'filter !imp': '.card{filter:drop-shadow(300px 300px 0 red)!important;background:red}', 'container-type': '.card{container-type:size}', 'columns': '.card{columns:100;column-gap:-100px;width:9999px;background:red}',
};
const dos = {
  'deep nest': '.a{'.repeat(2000) + 'color:red' + '}'.repeat(2000), 'deep :is': ':is('.repeat(3000) + '.a' + ')'.repeat(3000) + '{color:red}', 'big sheet 2MB': ('.a' + ',.a'.repeat(50000) + '{color:red}\n').repeat(10),
  'anim cpu': '@keyframes k{to{filter:blur(50px)}}.card{animation:k 1ms infinite}',
};

// The second review's escape (architect#601): what the URL parser reads as
// `..` or as somewhere else, which a text match does not see. Every one is
// refused; every property that loads an image is tried.
const traversal = {};
const family = [
  `${P}%2e%2e/%2e%2e/%2e%2e/s/secret-dd`, `"${P}.%2E/.%2e/.%2e/s/secret-dd2"`, `${P}%2E./%2E./x`, `${P}%2E%2E/x`, `${P}.%2e/x`,
  `${P}%252e%252e/x`, `${P}%2f..%2fx`, `"${P}..\\\\..\\\\x"`, `${P}./../x`, `${P}a/../../x`, `"${P}%09../x"`, `"${P}\\9 ../x"`,
  `//127.0.0.1/x`, `"//e.vil${P}a.png"`, `${P}a.png?x`, `"${P}a.png#x"`, `"${P}a\\9 b.png"`, `"${P}a\\a b.png"`,
  `"http://127.0.0.1:1${P}a.png"`, `"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg'/>"`, `"blob:${P}a.png"`, `/_ext/x`, `/x`,
];
for (const u of family) for (const prop of ['background-image', 'list-style', 'mask', 'mask-image', 'shape-outside', 'border-image', 'content', 'cursor'])
  traversal[`traversal ${prop}: ${u}`] = `.card,.card::before{${prop}:url(${u})}`;

/// [name, css] of every payload: the second review's 397 and the traversal
/// family. Kept or refused, none may reach past its box or request
/// anything but the extension's own files.
export const PAYLOADS = Object.entries({ ...esc, ...comments, ...eof, ...markup, ...sel, ...at, ...vals, ...layout, ...dos, ...traversal });
/// The traversal family's names: each must be refused.
export const TRAVERSAL = Object.keys(traversal);
