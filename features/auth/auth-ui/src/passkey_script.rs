//! The one place this UI needs JavaScript.
//!
//! `navigator.credentials` is the whole of `WebAuthn` and there is no
//! form post that reaches it — a passkey is created and used by the
//! browser's own credential store, not by anything a server can render.
//! The same is true of `window.ethereum`: a wallet signature comes from
//! an extension, not from a form. So this is a genuine exception to the
//! rest of these pages, and it is kept to exactly that: the script adds
//! passkey and wallet buttons and does nothing else.
//!
//! # Progressive enhancement, not a dependency
//!
//! Every passkey control is marked `hidden` in the HTML and revealed by
//! the script only after it has checked that `PublicKeyCredential`
//! exists. A browser with no JavaScript, or an older one, therefore
//! shows a page with no passkey affordances at all rather than buttons
//! that do nothing — and everything else on the page keeps working,
//! because everything else is still a form.
//!
//! # What the conversions are for
//!
//! `WebAuthn` deals in `ArrayBuffer`s; JSON does not. The server sends
//! and expects base64url without padding — what `webauthn-rs`
//! serialises — so the script decodes the challenge, the user handle
//! and any credential ids on the way in, and encodes the attestation,
//! the client data and the signature on the way out. Newer browsers
//! offer `parseCreationOptionsFromJSON`/`toJSON` for this, but support
//! is uneven enough that hand-rolling thirty lines is the smaller risk.

/// The script, inlined into any page with passkey controls.
pub const PASSKEY_SCRIPT: &str = r#"
(function () {
  if (!window.PublicKeyCredential || !navigator.credentials) return;

  var b64urlToBytes = function (value) {
    var padded = value.replace(/-/g, '+').replace(/_/g, '/');
    while (padded.length % 4) padded += '=';
    var binary = atob(padded);
    var bytes = new Uint8Array(binary.length);
    for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  };

  var bytesToB64url = function (buffer) {
    var bytes = new Uint8Array(buffer);
    var binary = '';
    for (var i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
    return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  };

  var post = function (url, body) {
    return fetch(url, {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body || {})
    }).then(function (response) {
      return response.json().then(function (json) {
        if (!response.ok) throw new Error(json.error || 'Something went wrong.');
        return json;
      });
    });
  };

  var say = function (id, message, bad) {
    var el = document.getElementById(id);
    if (!el) return;
    el.textContent = message;
    el.className = bad ? 'error' : 'ok';
    el.setAttribute('role', bad ? 'alert' : 'status');
    el.hidden = false;
  };

  // Show every passkey control now that we know the browser has the API.
  var reveal = function () {
    var hidden = document.querySelectorAll('[data-passkey]');
    for (var i = 0; i < hidden.length; i++) hidden[i].hidden = false;
  };

  var register = function (button) {
    button.disabled = true;
    var name = document.getElementById('passkey-name');
    post('/account/passkeys/begin')
      .then(function (start) {
        var options = JSON.parse(start.options).publicKey;
        options.challenge = b64urlToBytes(options.challenge);
        options.user.id = b64urlToBytes(options.user.id);
        (options.excludeCredentials || []).forEach(function (c) {
          c.id = b64urlToBytes(c.id);
        });
        return navigator.credentials.create({ publicKey: options }).then(function (credential) {
          var transports = null;
          if (credential.response.getTransports) {
            try { transports = credential.response.getTransports(); } catch (e) { transports = null; }
          }
          return post('/account/passkeys/complete', {
            handle: start.handle,
            name: name && name.value ? name.value : 'Passkey',
            credential: {
              id: credential.id,
              rawId: bytesToB64url(credential.rawId),
              type: credential.type,
              extensions: {},
              response: {
                attestationObject: bytesToB64url(credential.response.attestationObject),
                clientDataJSON: bytesToB64url(credential.response.clientDataJSON),
                transports: transports
              }
            }
          });
        });
      })
      .then(function () { window.location.reload(); })
      .catch(function (error) {
        button.disabled = false;
        // A person who changed their mind is not an error worth shouting
        // about; the browser reports that as NotAllowedError, the same
        // as a timeout.
        if (error && error.name === 'NotAllowedError') {
          say('passkey-status', 'No passkey was created.', false);
        } else {
          say('passkey-status', error.message || 'Could not create a passkey.', true);
        }
      });
  };

  var signIn = function (button) {
    button.disabled = true;
    var emailField = document.getElementById('email');
    var email = emailField && emailField.value ? emailField.value : null;
    var returnTo = button.getAttribute('data-return-to') || '/';
    post('/login/passkey/begin', { email: email })
      .then(function (start) {
        var options = JSON.parse(start.options).publicKey;
        options.challenge = b64urlToBytes(options.challenge);
        (options.allowCredentials || []).forEach(function (c) {
          c.id = b64urlToBytes(c.id);
        });
        return navigator.credentials.get({ publicKey: options }).then(function (credential) {
          return post('/login/passkey/complete', {
            handle: start.handle,
            return_to: returnTo,
            credential: {
              id: credential.id,
              rawId: bytesToB64url(credential.rawId),
              type: credential.type,
              extensions: {},
              response: {
                authenticatorData: bytesToB64url(credential.response.authenticatorData),
                clientDataJSON: bytesToB64url(credential.response.clientDataJSON),
                signature: bytesToB64url(credential.response.signature),
                userHandle: credential.response.userHandle
                  ? bytesToB64url(credential.response.userHandle)
                  : null
              }
            }
          });
        });
      })
      .then(function (done) { window.location.assign(done.redirect || '/'); })
      .catch(function (error) {
        button.disabled = false;
        if (error && error.name === 'NotAllowedError') {
          say('passkey-status', 'No passkey was used.', false);
        } else {
          say('passkey-status', error.message || 'Could not sign in with a passkey.', true);
        }
      });
  };

  var wallet = function (button) {
    if (!window.ethereum) {
      say('passkey-status', 'No wallet was found in this browser.', true);
      return;
    }
    button.disabled = true;
    var returnTo = button.getAttribute('data-return-to') || '/';
    window.ethereum
      .request({ method: 'eth_requestAccounts' })
      .then(function (accounts) {
        var address = accounts && accounts[0];
        if (!address) throw new Error('No account was shared.');
        return post('/login/wallet/begin', {}).then(function (start) {
          // The message is built by the SERVER and signed as-is. A
          // message assembled here could be edited before signing, and
          // the domain line is what stops a signature obtained on one
          // site being replayed on another.
          return window.ethereum
            .request({ method: 'personal_sign', params: [start.message, address] })
            .then(function (signature) {
              return post('/login/wallet/complete', {
                message: start.message,
                signature: signature,
                return_to: returnTo
              });
            });
        });
      })
      .then(function (done) { window.location.assign(done.redirect || '/'); })
      .catch(function (error) {
        button.disabled = false;
        // 4001 is the wallet's code for "the person said no".
        if (error && (error.code === 4001 || error.name === 'NotAllowedError')) {
          say('passkey-status', 'No wallet was used.', false);
        } else {
          say('passkey-status', error.message || 'Could not sign in with a wallet.', true);
        }
      });
  };

  document.addEventListener('DOMContentLoaded', function () {
    reveal();
    var connect = document.getElementById('wallet-signin');
    if (connect) {
      // Only offered where a wallet actually exists.
      if (window.ethereum) {
        connect.parentNode.hidden = false;
        connect.addEventListener('click', function () { wallet(connect); });
      }
    }
    var add = document.getElementById('passkey-register');
    if (add) add.addEventListener('click', function () { register(add); });
    var use = document.getElementById('passkey-signin');
    if (use) use.addEventListener('click', function () { signIn(use); });
  });
})();
"#;
