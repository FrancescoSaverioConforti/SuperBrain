// Real cross-file call site, mirroring request/request's own
// request.js:39 `cookies.jar()` / index.js:80 `cookies.parse(str)`
// shape: a receiver-qualified call to a function that only exists via
// the exports.X = function(){} idiom in another file.
const cookies = require('./commonjs_exports_property_function');

function useBoth() {
  return cookies.jar(1) + cookies.parse('x');
}
