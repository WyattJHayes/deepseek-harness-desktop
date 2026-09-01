//! 剪贴板图片回退桥：让 Linux/WebKitGTK 下 iframe 的「贴图」走原生剪贴板。
//!
//! WebKitGTK 不通过 Web API 暴露 `image/*` 剪贴板条目，因此截图复制到系统剪贴板后，
//! 在桌面端内嵌的 dsh iframe 中输入框按 Ctrl+V 时，`paste` 事件的
//! `clipboardData` 里既没有图片也没有文本。本脚本在**捕获阶段**监听 `paste`，
//! 检测到「无图片且无文本」时：preventDefault → 向宿主 postMessage 请求读取系统
//! 剪贴板图片（宿主调用 `bridge::read_clipboard_image`）→ 拿到 PNG data URL →
//! 构造 File 并重发一次合成 `paste` 事件给当前焦点元素，让 dsh 聊天框按普通
//! 贴图路径处理，从而与浏览器行为一致。
//!
//! 与 [`crate::desktop::notification::NOTIFICATION_SHIM_JS`] / [`crate::desktop::nav::NAV_SHIM_JS`]
//! 走同一套文档创建前注入通道（Tauri 的 `initialization_script_for_all_frames`，Windows
//! 底层映射为 WebView2 的 `AddScriptToExecuteOnDocumentCreated`）。脚本带 `__dsh_clipboard_image_bridge__`
//! 幂等守卫，重复注入安全；只处理「直接 iframe」发来的剪贴板请求，避免多层 iframe 误转发。

/// 构造带会话密钥的剪贴板回退脚本。
///
/// 密钥只通过初始化脚本交给闭包；脚本在页面代码运行前捕获原生加密 API 并导入
/// 不可导出的密钥，之后只会在真实粘贴事件里生成 nonce 并计算证明。宿主收到请求后
/// 还会在 Rust 侧校验 HMAC 和 nonce 的单次消费状态。
pub(crate) fn paste_shim_js(secret: &str) -> String {
    let secret_literal =
        serde_json::to_string(secret).expect("clipboard bridge secret should be serializable");
    r#"(function () {
  if (window.__dsh_clipboard_image_bridge__) return;
  window.__dsh_clipboard_image_bridge__ = true;

  // 请求方向：iframe → 宿主；响应方向：宿主 → iframe
  var REQ_SRC = 'dsh-clipboard-image-bridge';
  var RES_SRC = 'dsh-desktop-clipboard';
  var REQ_TYPE = 'dsh://clipboard-image:read';
  var BRIDGE_SECRET_B64 = __DSH_CLIPBOARD_BRIDGE_SECRET__;
  var SIGNING_CONTEXT = 'dsh-clipboard-image-v1:';
  var MAX_PENDING = 32;
  var REQUEST_TIMEOUT_MS = 5000;

  // 初始化脚本先于页面脚本执行；捕获原生引用，避免页面改写 Web Crypto 后
  // 在真实粘贴发生时截获原始密钥或签名输入。
  var NativePromise = window.Promise;
  var NativePromiseThen = NativePromise && NativePromise.prototype && NativePromise.prototype.then;
  var NativePromiseReject = NativePromise && typeof NativePromise.reject === 'function'
    ? NativePromise.reject.bind(NativePromise)
    : null;
  var NativeUint8Array = window.Uint8Array;
  var NativeAtob = typeof window.atob === 'function' ? window.atob.bind(window) : null;
  var NativeBtoa = typeof window.btoa === 'function' ? window.btoa.bind(window) : null;
  var NativeTextEncoder = window.TextEncoder;
  var NativeTextEncoderEncode = NativeTextEncoder && NativeTextEncoder.prototype
    ? NativeTextEncoder.prototype.encode
    : null;
  var NativeDate = window.Date;
  var NativeDateNow = NativeDate && typeof NativeDate.now === 'function'
    ? NativeDate.now.bind(NativeDate)
    : null;
  var NativeString = typeof window.String === 'function' ? window.String : null;
  var NativeStringFromCharCode = NativeString && typeof NativeString.fromCharCode === 'function'
    ? NativeString.fromCharCode.bind(NativeString)
    : null;
  var NativeSetTimeout = typeof window.setTimeout === 'function'
    ? window.setTimeout.bind(window)
    : null;
  var NativeClearTimeout = typeof window.clearTimeout === 'function'
    ? window.clearTimeout.bind(window)
    : null;
  var NativeCrypto = window.crypto;
  var NativeGetRandomValues = NativeCrypto && typeof NativeCrypto.getRandomValues === 'function'
    ? NativeCrypto.getRandomValues.bind(NativeCrypto)
    : null;
  var NativeSubtle = NativeCrypto && NativeCrypto.subtle;
  var NativeImportKey = NativeSubtle && typeof NativeSubtle.importKey === 'function'
    ? NativeSubtle.importKey.bind(NativeSubtle)
    : null;
  var NativeSign = NativeSubtle && typeof NativeSubtle.sign === 'function'
    ? NativeSubtle.sign.bind(NativeSubtle)
    : null;

  var reqSeq = 0;
  var hmacKeyPromise = null;
  var pending = {}; // id -> { resolve, target, timeout }

  function resolvePending(id, value) {
    var item = pending[id];
    if (!item) return;
    delete pending[id];
    if (NativeClearTimeout && item.timeout !== null) NativeClearTimeout(item.timeout);
    item.resolve(value);
  }

  function bytesToHex(bytes) {
    var result = '';
    for (var i = 0; i < bytes.length; i++) {
      result += ('0' + bytes[i].toString(16)).slice(-2);
    }
    return result;
  }

  function bytesToBase64(buffer) {
    if (!NativeUint8Array || !NativeBtoa || !NativeStringFromCharCode) {
      throw new Error('clipboard bridge base64 unavailable');
    }
    var bytes = new NativeUint8Array(buffer);
    var binary = '';
    for (var i = 0; i < bytes.length; i++) binary += NativeStringFromCharCode(bytes[i]);
    return NativeBtoa(binary);
  }

  function secretBytes() {
    if (!NativeAtob || !NativeUint8Array) throw new Error('clipboard bridge secret unavailable');
    var binary = NativeAtob(BRIDGE_SECRET_B64);
    var bytes = new NativeUint8Array(binary.length);
    for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  }

  function newNonce() {
    if (!NativeGetRandomValues || !NativeUint8Array) return null;
    var bytes = new NativeUint8Array(16);
    try {
      NativeGetRandomValues(bytes);
      return bytesToHex(bytes);
    } catch (_) {
      return null;
    }
  }

  function rejectedPromise(message) {
    return NativePromiseReject ? NativePromiseReject(new Error(message)) : null;
  }

  function signNonce(nonce, issuedAtText) {
    if (!hmacKeyPromise || !NativePromiseThen || !NativeSign
      || !NativeTextEncoder || !NativeTextEncoderEncode) {
      return rejectedPromise('clipboard bridge crypto unavailable');
    }
    var encoded;
    try {
      encoded = NativeTextEncoderEncode.call(
        new NativeTextEncoder(),
        SIGNING_CONTEXT + nonce + ':' + issuedAtText
      );
    } catch (_) {
      return rejectedPromise('clipboard bridge crypto unavailable');
    }
    var signed = NativePromiseThen.call(hmacKeyPromise, function (key) {
      return NativeSign('HMAC', key, encoded);
    });
    return NativePromiseThen.call(signed, bytesToBase64);
  }

  // 在页面脚本有机会改写全局对象前导入密钥；CryptoKey 不可导出，页面只能看到
  // 后续真实粘贴产生的单次证明，不能借此伪造新的 nonce。
  if (NativeImportKey && NativePromiseReject) {
    try {
      hmacKeyPromise = NativeImportKey(
        'raw',
        secretBytes(),
        { name: 'HMAC', hash: 'SHA-256' },
        false,
        ['sign']
      );
    } catch (_) {
      hmacKeyPromise = rejectedPromise('clipboard bridge crypto unavailable');
    }
  }

  function requestClipboardImage(target) {
    if (!NativePromise || !NativePromiseThen) return null;
    return new NativePromise(function (resolve) {
      if (Object.keys(pending).length >= MAX_PENDING) {
        resolve(null);
        return;
      }
      reqSeq += 1;
      var id = 'req-' + reqSeq;
      var nonce = newNonce();
      if (!nonce) {
        resolve(null);
        return;
      }
      if (!NativeDateNow || !NativeString) {
        resolve(null);
        return;
      }
      var issuedAt = NativeDateNow();
      var issuedAtText = NativeString(issuedAt);
      pending[id] = { resolve: resolve, target: target, timeout: null };
      if (NativeSetTimeout) {
        pending[id].timeout = NativeSetTimeout(function () {
          resolvePending(id, null);
        }, REQUEST_TIMEOUT_MS);
      }
      var signed = signNonce(nonce, issuedAtText);
      if (!signed) {
        resolvePending(id, null);
        return;
      }
      NativePromiseThen.call(signed, function (proof) {
        if (!pending[id]) return;
        try {
          window.parent.postMessage(
            {
              source: REQ_SRC,
              type: REQ_TYPE,
              id: id,
              nonce: nonce,
              issued_at: issuedAt,
              proof: proof
            },
            '*'
          );
        } catch (_) {
          resolvePending(id, null);
        }
      }, function () {
        resolvePending(id, null);
      });
    });
  }

  // 宿主回包
  window.addEventListener('message', function (event) {
    var data = event.data;
    if (!data || typeof data !== 'object' || data.source !== RES_SRC) return;
    if (event.source !== window.parent) return;
    var item = pending[data.id];
    if (!item) return;
    resolvePending(data.id, data.data_url || null);
  });

  // 剪贴板事件里是否已带图片（非 WebKitGTK 场景）：不介入，交 dsh 自身处理
  function hasImageData(dt) {
    if (!dt) return false;
    try {
      if (dt.items) {
        for (var i = 0; i < dt.items.length; i++) {
          var it = dt.items[i];
          if (it && it.type && it.type.indexOf('image/') === 0) return true;
        }
      }
      if (dt.files && dt.files.length > 0) return true;
      var types = dt.types || [];
      for (var j = 0; j < types.length; j++) {
        if (types[j] === 'Files') return true;
      }
    } catch (_) {}
    return false;
  }

  // 剪贴板事件里是否带文本/HTML（普通文本复制）：不介入，让 dsh 正常粘贴文本
  function hasTextData(dt) {
    if (!dt) return false;
    try {
      if (dt.getData('text/plain')) return true;
      if (dt.getData('text/html')) return true;
      var types = dt.types || [];
      return types.indexOf('text/plain') !== -1 || types.indexOf('text/html') !== -1;
    } catch (_) {
      return false;
    }
  }

  function isPasteTarget(target) {
    return !!(target && (target.isContentEditable || /^(textarea|INPUT)$/i.test(target.tagName)));
  }

  function dataUrlToBlob(dataUrl) {
    var comma = dataUrl.indexOf(',');
    if (comma === -1) return null;
    var header = dataUrl.slice(0, comma);
    var mime = (header.match(/:(.*?);/) || [])[1] || 'image/png';
    var b64 = dataUrl.slice(comma + 1);
    try {
      var bin = atob(b64);
      var bytes = new Uint8Array(bin.length);
      for (var i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      return new Blob([bytes], { type: mime });
    } catch (_) {
      return null;
    }
  }

  // 把图片作为 File 塞进新的 DataTransfer，再派发合成 paste 给焦点元素
  function dispatchImagePaste(target, dataUrl) {
    var blob = dataUrlToBlob(dataUrl);
    if (!blob) return;
    var file = new File([blob], 'clipboard-image.png', { type: blob.type || 'image/png' });
    try {
      var dt = new DataTransfer();
      dt.items.add(file);
      var ev;
      try {
        ev = new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true });
      } catch (_) {
        // 个别 WebKit 版本 ClipboardEvent 构造器不支持 clipboardData：用普通事件补属性
        ev = new Event('paste', { bubbles: true, cancelable: true });
        Object.defineProperty(ev, 'clipboardData', { value: dt });
      }
      if (target.isConnected) {
        target.dispatchEvent(ev);
      } else {
        var active = document.activeElement;
        if (active && isPasteTarget(active)) active.dispatchEvent(ev);
      }
    } catch (_) {}
  }

  // 捕获阶段先于 dsh 应用自身监听：空剪贴板时接管并走原生读取。
  // 仅当本窗口是子 frame（dsh iframe）时才接管；顶层壳层文档里没有聊天框，
  // 且 `window.parent` 指向自身，postMessage 不会命中宿主（见 handleClipboardImage），
  // 这里直接把拦截限定在子 frame，避免对壳层输入产生任何干扰。
  if (window !== window.parent) {
    document.addEventListener('paste', function (event) {
      var dt = event.clipboardData;
      if (hasImageData(dt)) return;
      if (hasTextData(dt)) return;
      if (event.isTrusted !== true) return;

      var target = document.activeElement || event.target;
      if (!isPasteTarget(target)) return;

      event.preventDefault();
      requestClipboardImage(target).then(function (dataUrl) {
        if (dataUrl) dispatchImagePaste(target, dataUrl);
      });
    }, true);
  }
})();"#
        .replace("__DSH_CLIPBOARD_BRIDGE_SECRET__", &secret_literal)
}
