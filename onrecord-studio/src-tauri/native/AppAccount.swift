import AppKit
import AuthenticationServices
import CryptoKit
import Darwin
import Foundation
import Security

// Direct-distribution account layer. C ABI consumed by the Rust `direct-license` feature; every
// exported function returns a malloc'd UTF-8 JSON string released with
// onrecord_account_free_string. Nothing here logs: tokens, codes, verifiers, emails and keys never
// leave the return value or the Keychain.

// MARK: - Constants

enum AccountConfig {
    static let authHost = "auth.writerslogic.com"
    static let callbackScheme = "com.onrecord.studio"
    static let callbackHost = "auth"
    static let callbackPath = "/callback"
    static let keychainService = "com.onrecord.studio.account"
    static let sessionAccount = "session"
    static let deviceKeyTag = Data("com.onrecord.studio.account.device-key".utf8)
    static let bundleIdentifier = "com.onrecord.studio"
    static let teamIdentifier = "U3PZN7P3E5"
    static let anonKeyInfoKey = "ONRECORD_SUPABASE_ANON_KEY"
    static let refreshMarginSeconds: Int64 = 60
    static let signInTimeoutSeconds: TimeInterval = 300
    static let maxResponseBytes = 256 * 1024
    static let maxTokenLength = 16 * 1024
}

// MARK: - Pure helpers

enum AccountCodec {
    static func base64URL(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    static func decodeBase64URL(_ text: String) -> Data? {
        guard !text.isEmpty, text.utf8.allSatisfy({ isBase64URLByte($0) }) else { return nil }
        var base64 = text.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        switch base64.count % 4 {
        case 0: break
        case 2: base64 += "=="
        case 3: base64 += "="
        default: return nil
        }
        return Data(base64Encoded: base64)
    }

    private static func isBase64URLByte(_ byte: UInt8) -> Bool {
        (byte >= 0x41 && byte <= 0x5A) || (byte >= 0x61 && byte <= 0x7A)
            || (byte >= 0x30 && byte <= 0x39) || byte == 0x2D || byte == 0x5F
    }

    static func randomBytes(_ count: Int) -> Data? {
        var bytes = [UInt8](repeating: 0, count: count)
        guard SecRandomCopyBytes(kSecRandomDefault, count, &bytes) == errSecSuccess else { return nil }
        return Data(bytes)
    }

    /// RFC 7636 section 4.2, S256.
    static func pkceChallenge(verifier: String) -> String {
        base64URL(Data(SHA256.hash(data: Data(verifier.utf8))))
    }

    static func constantTimeEqual(_ a: String, _ b: String) -> Bool {
        let x = Array(a.utf8), y = Array(b.utf8)
        guard x.count == y.count else { return false }
        var diff: UInt8 = 0
        for i in 0..<x.count { diff |= x[i] ^ y[i] }
        return diff == 0
    }

    /// Everything outside RFC 3986 unreserved is escaped, so a redirect_to that itself carries a
    /// query survives as one parameter value.
    static func percentEncode(_ value: String) -> String {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~")
        return value.addingPercentEncoding(withAllowedCharacters: allowed) ?? ""
    }

    static func thumbprint(publicKeyX963: Data) -> String {
        base64URL(Data(SHA256.hash(data: publicKeyX963)))
    }
}

enum OAuthProvider {
    /// Client-facing name to the Supabase provider id. Email sign-in is a separate flow.
    static func supabaseID(for name: String) -> String? {
        switch name {
        case "google": return "google"
        case "apple": return "apple"
        case "github": return "github"
        case "linkedin", "linkedin_oidc": return "linkedin_oidc"
        default: return nil
        }
    }
}

enum CallbackResult: Equatable {
    case code(String)
    case providerError(String)
    case invalid(String)
}

enum OAuthURLs {
    static func redirectTo(state: String) -> String {
        "\(AccountConfig.callbackScheme)://\(AccountConfig.callbackHost)\(AccountConfig.callbackPath)?state=\(state)"
    }

    static func authorizeURL(provider: String, state: String, challenge: String) -> URL? {
        let query = [
            "provider=\(AccountCodec.percentEncode(provider))",
            "redirect_to=\(AccountCodec.percentEncode(redirectTo(state: state)))",
            "code_challenge=\(AccountCodec.percentEncode(challenge))",
            "code_challenge_method=s256",
        ].joined(separator: "&")
        return URL(string: "https://\(AccountConfig.authHost)/auth/v1/authorize?\(query)")
    }

    static func validateCallback(_ url: URL, expectedState: String) -> CallbackResult {
        guard let parts = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            return .invalid("Callback URL is malformed.")
        }
        guard parts.scheme?.lowercased() == AccountConfig.callbackScheme,
              parts.host?.lowercased() == AccountConfig.callbackHost,
              parts.path == AccountConfig.callbackPath,
              parts.user == nil, parts.password == nil, parts.port == nil
        else {
            return .invalid("Callback URL has an unexpected scheme, host, or path.")
        }
        let items = parts.queryItems ?? []
        let states = items.filter { $0.name == "state" }
        guard states.count == 1, let state = states[0].value,
              AccountCodec.constantTimeEqual(state, expectedState)
        else {
            return .invalid("Callback state did not match this sign-in attempt.")
        }
        let errors = items.filter { $0.name == "error" }
        if let fragment = parts.fragment, !fragment.isEmpty {
            // Tokens in a fragment are the implicit flow, which this client never accepts.
            if fragment.contains("access_token") || fragment.contains("refresh_token") {
                return .invalid("Callback carried tokens directly.")
            }
        }
        if let first = errors.first {
            let name = first.value ?? ""
            let safe = name.count <= 64 && name.allSatisfy { $0.isASCII && ($0.isLetter || $0 == "_") }
            return .providerError(safe && !name.isEmpty ? name : "unknown")
        }
        let codes = items.filter { $0.name == "code" }
        guard codes.count == 1, let code = codes[0].value, !code.isEmpty, code.count <= 512,
              code.utf8.allSatisfy({ $0 > 0x20 && $0 < 0x7F })
        else {
            return .invalid("Callback carried no usable authorization code.")
        }
        return .code(code)
    }
}

// MARK: - Session model

struct StoredSession: Codable, Equatable {
    var accessToken: String
    var refreshToken: String
    var expiresAt: Int64
    var userId: String
    var email: String?
    var provider: String?
}

private struct TokenResponse: Decodable {
    struct User: Decodable {
        struct AppMetadata: Decodable { let provider: String? }
        let id: String
        let email: String?
        let app_metadata: AppMetadata?
    }
    let access_token: String
    let refresh_token: String
    let token_type: String?
    let expires_in: Int64?
    let expires_at: Int64?
    let user: User
}

enum TokenParsing {
    /// Parses a Supabase token endpoint body without reading the JWT itself.
    static func session(from body: Data, now: Date, fallbackProvider: String?) -> StoredSession? {
        guard let response = try? JSONDecoder().decode(TokenResponse.self, from: body) else { return nil }
        guard response.token_type?.lowercased() == "bearer" || response.token_type == nil,
              !response.access_token.isEmpty, response.access_token.count <= AccountConfig.maxTokenLength,
              !response.refresh_token.isEmpty, response.refresh_token.count <= AccountConfig.maxTokenLength,
              !response.user.id.isEmpty, response.user.id.count <= 128
        else { return nil }
        let nowSeconds = Int64(now.timeIntervalSince1970)
        let expiresAt: Int64
        if let at = response.expires_at {
            expiresAt = at
        } else if let seconds = response.expires_in {
            expiresAt = nowSeconds + seconds
        } else {
            return nil
        }
        // A token that is already dead, or one valid for over 30 days, is not a Supabase access token.
        guard expiresAt > nowSeconds, expiresAt - nowSeconds <= 30 * 86_400 else { return nil }
        return StoredSession(
            accessToken: response.access_token,
            refreshToken: response.refresh_token,
            expiresAt: expiresAt,
            userId: response.user.id,
            email: response.user.email,
            provider: response.user.app_metadata?.provider ?? fallbackProvider
        )
    }
}

// MARK: - JSON output

private func respond(_ object: [String: Any]) -> UnsafeMutablePointer<CChar>? {
    if let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]),
       let text = String(data: data, encoding: .utf8) {
        return strdup(text)
    }
    return strdup("{\"outcome\":\"failed\",\"message\":\"The account layer produced an unreadable result.\"}")
}

private func failure(_ message: String) -> [String: Any] {
    ["outcome": "failed", "message": message]
}

private func sessionSummary(_ session: StoredSession, outcome: String = "signed_in") -> [String: Any] {
    var out: [String: Any] = ["outcome": outcome, "message": "Signed in.", "userId": session.userId]
    if let email = session.email { out["email"] = email }
    if let provider = session.provider { out["provider"] = provider }
    return out
}

private func keychainMessage(_ status: OSStatus) -> String {
    if status == errSecMissingEntitlement {
        return "The Keychain refused access: this build lacks the keychain-access-groups entitlement and provisioning profile."
    }
    return "Keychain error (OSStatus \(status))."
}

// MARK: - Keychain session store

enum SessionStore {
    private static var base: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: AccountConfig.keychainService,
            kSecAttrAccount as String: AccountConfig.sessionAccount,
            kSecUseDataProtectionKeychain as String: true,
            kSecAttrSynchronizable as String: false,
        ]
    }

    enum LoadResult {
        case found(StoredSession)
        case absent
        case error(OSStatus)
    }

    static func load() -> LoadResult {
        var query = base
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        if status == errSecItemNotFound { return .absent }
        guard status == errSecSuccess, let data = item as? Data else { return .error(status) }
        guard let session = try? JSONDecoder().decode(StoredSession.self, from: data) else {
            // An unreadable record is unusable state; dropping it forces a clean sign-in.
            _ = clear()
            return .absent
        }
        return .found(session)
    }

    static func save(_ session: StoredSession) -> OSStatus {
        guard let data = try? JSONEncoder().encode(session) else { return errSecParam }
        let update = SecItemUpdate(
            base as CFDictionary,
            [kSecValueData as String: data] as CFDictionary
        )
        if update != errSecItemNotFound { return update }
        var add = base
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        return SecItemAdd(add as CFDictionary, nil)
    }

    @discardableResult
    static func clear() -> OSStatus {
        let status = SecItemDelete(base as CFDictionary)
        return status == errSecItemNotFound ? errSecSuccess : status
    }
}

// MARK: - Generic secure storage

enum SecureItems {
    static let allowedKeys: Set<String> = ["entitlement_token", "max_seen_time", "max_seen_now"]
    static let maxValueBytes = 16 * 1024

    static func isAllowed(key: String) -> Bool { allowedKeys.contains(key) }
    static func isWithinCap(_ value: String) -> Bool { value.utf8.count <= maxValueBytes }

    private static let lock = NSLock()

    private static func query(_ key: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: AccountConfig.keychainService,
            kSecAttrAccount as String: key,
            kSecUseDataProtectionKeychain as String: true,
            kSecAttrSynchronizable as String: false,
        ]
    }

    static func get(_ key: String) -> [String: Any] {
        guard isAllowed(key: key) else { return failure("Unsupported storage key.") }
        lock.lock()
        defer { lock.unlock() }
        var search = query(key)
        search[kSecReturnData as String] = true
        search[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(search as CFDictionary, &item)
        if status == errSecItemNotFound { return ["outcome": "missing"] }
        guard status == errSecSuccess, let data = item as? Data, let value = String(data: data, encoding: .utf8) else {
            return failure(keychainMessage(status == errSecSuccess ? errSecDecode : status))
        }
        return ["outcome": "ok", "value": value]
    }

    static func set(_ key: String, _ value: String) -> [String: Any] {
        guard isAllowed(key: key) else { return failure("Unsupported storage key.") }
        guard isWithinCap(value) else { return failure("The value exceeds the storage limit.") }
        lock.lock()
        defer { lock.unlock() }
        if value.isEmpty {
            let status = SecItemDelete(query(key) as CFDictionary)
            return status == errSecSuccess || status == errSecItemNotFound
                ? ["outcome": "ok"] : failure(keychainMessage(status))
        }
        let data = Data(value.utf8)
        let update = SecItemUpdate(query(key) as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        var status = update
        if update == errSecItemNotFound {
            var add = query(key)
            add[kSecValueData as String] = data
            add[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
            status = SecItemAdd(add as CFDictionary, nil)
        }
        return status == errSecSuccess ? ["outcome": "ok"] : failure(keychainMessage(status))
    }
}

// MARK: - Bounded HTTPS

private final class BoundedRequest: NSObject, URLSessionDataDelegate {
    private let semaphore = DispatchSemaphore(value: 0)
    private var body = Data()
    private var status = 0
    private var overflow = false
    private var failed = false

    struct Reply {
        let status: Int
        let body: Data
    }

    /// Redirects are refused: the apikey header must only ever reach the pinned host.
    func urlSession(
        _ session: URLSession, task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest,
        completionHandler: @escaping (URLRequest?) -> Void
    ) {
        completionHandler(nil)
    }

    func urlSession(
        _ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
        completionHandler: @escaping (URLSession.ResponseDisposition) -> Void
    ) {
        if let http = response as? HTTPURLResponse { status = http.statusCode }
        if response.expectedContentLength > Int64(AccountConfig.maxResponseBytes) {
            overflow = true
            completionHandler(.cancel)
            return
        }
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        body.append(data)
        if body.count > AccountConfig.maxResponseBytes {
            overflow = true
            dataTask.cancel()
        }
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        failed = error != nil
        semaphore.signal()
    }

    static func post(path: String, query: String, json: [String: String], anonKey: String, bearer: String? = nil) -> Reply? {
        guard let url = URL(string: "https://\(AccountConfig.authHost)\(path)?\(query)"),
              url.scheme == "https",
              let payload = try? JSONSerialization.data(withJSONObject: json)
        else { return nil }
        var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 20)
        request.httpMethod = "POST"
        request.httpBody = payload
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue(anonKey, forHTTPHeaderField: "apikey")
        if let bearer { request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization") }

        let configuration = URLSessionConfiguration.ephemeral
        configuration.tlsMinimumSupportedProtocolVersion = .TLSv13
        configuration.timeoutIntervalForRequest = 20
        configuration.timeoutIntervalForResource = 30
        configuration.urlCache = nil
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        let delegate = BoundedRequest()
        let session = URLSession(configuration: configuration, delegate: delegate, delegateQueue: nil)
        defer { session.finishTasksAndInvalidate() }
        session.dataTask(with: request).resume()
        guard delegate.semaphore.wait(timeout: .now() + 35) == .success else {
            session.invalidateAndCancel()
            return nil
        }
        if delegate.failed || delegate.overflow || delegate.status == 0 { return nil }
        return Reply(status: delegate.status, body: delegate.body)
    }
}

// MARK: - Account service

private enum AnonKey {
    static func read() -> String? {
        guard let value = Bundle.main.object(forInfoDictionaryKey: AccountConfig.anonKeyInfoKey) as? String else {
            return nil
        }
        let key = value.trimmingCharacters(in: .whitespacesAndNewlines)
        // An unfilled build placeholder is a missing key, not a key.
        guard !key.isEmpty, key.count <= 512, !key.hasPrefix("$("), !key.hasPrefix("__") else { return nil }
        return key
    }

    static let missingMessage =
        "This build has no Supabase anon key (Info.plist \(AccountConfig.anonKeyInfoKey)); run prepare:direct with ONRECORD_SUPABASE_ANON_KEY."
}

private final class AttemptBox: @unchecked Sendable {
    let semaphore = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var done = false
    private(set) var result: WebAuthOutcome = .failed("Sign-in did not complete.")
    var session: ASWebAuthenticationSession?
    var anchorProvider: AnchorProvider?

    func finish(_ outcome: WebAuthOutcome) {
        lock.lock()
        defer { lock.unlock() }
        guard !done else { return }
        done = true
        result = outcome
        session = nil
        anchorProvider = nil
        semaphore.signal()
    }
}

private enum WebAuthOutcome {
    case callback(URL)
    case cancelled
    case failed(String)
}

private final class AnchorProvider: NSObject, ASWebAuthenticationPresentationContextProviding {
    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        NSApp.keyWindow ?? NSApp.mainWindow ?? NSApp.windows.first ?? ASPresentationAnchor()
    }
}

enum AccountService {
    private static let lock = NSLock()
    private static var signInActive = false

    // MARK: sign in

    static func signIn(providerName: String) -> [String: Any] {
        guard let provider = OAuthProvider.supabaseID(for: providerName) else {
            return failure("Unsupported sign-in provider.")
        }
        guard let anonKey = AnonKey.read() else { return failure(AnonKey.missingMessage) }
        guard !Thread.isMainThread else { return failure("Sign-in must not be started from the main thread.") }

        lock.lock()
        if signInActive {
            lock.unlock()
            return failure("A sign-in is already in progress.")
        }
        signInActive = true
        lock.unlock()
        defer {
            lock.lock()
            signInActive = false
            lock.unlock()
        }

        guard let verifierBytes = AccountCodec.randomBytes(64), let stateBytes = AccountCodec.randomBytes(32) else {
            return failure("Secure random generation failed.")
        }
        let verifier = AccountCodec.base64URL(verifierBytes)
        let state = AccountCodec.base64URL(stateBytes)
        let challenge = AccountCodec.pkceChallenge(verifier: verifier)
        guard let authorize = OAuthURLs.authorizeURL(provider: provider, state: state, challenge: challenge) else {
            return failure("Could not build the sign-in URL.")
        }

        let callback: URL
        switch runWebSession(url: authorize) {
        case .cancelled: return ["outcome": "cancelled", "message": "Sign-in was cancelled."]
        case .failed(let message): return failure(message)
        case .callback(let url): callback = url
        }

        let code: String
        switch OAuthURLs.validateCallback(callback, expectedState: state) {
        case .invalid(let message): return failure(message)
        case .providerError(let name): return failure("The provider reported an error (\(name)).")
        case .code(let value): code = value
        }

        guard let reply = BoundedRequest.post(
            path: "/auth/v1/token", query: "grant_type=pkce",
            json: ["auth_code": code, "code_verifier": verifier], anonKey: anonKey
        ) else {
            return failure("The sign-in service could not be reached.")
        }
        guard reply.status == 200 else {
            return failure("The sign-in service rejected the code exchange (HTTP \(reply.status)).")
        }
        guard let session = TokenParsing.session(from: reply.body, now: Date(), fallbackProvider: provider) else {
            return failure("The sign-in service returned an unreadable session.")
        }

        lock.lock()
        defer { lock.unlock() }
        let status = SessionStore.save(session)
        guard status == errSecSuccess else { return failure(keychainMessage(status)) }
        return sessionSummary(session)
    }

    private static func runWebSession(url: URL) -> WebAuthOutcome {
        let box = AttemptBox()
        Task { @MainActor in
            let anchor = AnchorProvider()
            let session = ASWebAuthenticationSession(
                url: url, callbackURLScheme: AccountConfig.callbackScheme
            ) { callbackURL, error in
                if let error {
                    if (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin {
                        box.finish(.cancelled)
                    } else {
                        box.finish(.failed("The browser sign-in session failed."))
                    }
                    return
                }
                if let callbackURL {
                    box.finish(.callback(callbackURL))
                } else {
                    box.finish(.failed("The browser sign-in session returned no callback."))
                }
            }
            session.presentationContextProvider = anchor
            session.prefersEphemeralWebBrowserSession = true
            box.session = session
            box.anchorProvider = anchor
            guard session.start() else {
                box.finish(.failed("The sign-in window could not be presented."))
                return
            }
            try? await Task.sleep(nanoseconds: UInt64(AccountConfig.signInTimeoutSeconds * 1_000_000_000))
            if box.session != nil {
                session.cancel()
                box.finish(.failed("Sign-in timed out."))
            }
        }
        let waited = box.semaphore.wait(timeout: .now() + AccountConfig.signInTimeoutSeconds + 30)
        if waited != .success { box.finish(.failed("Sign-in timed out.")) }
        return box.result
    }

    // MARK: session

    static func currentSession() -> [String: Any] {
        lock.lock()
        defer { lock.unlock() }
        switch SessionStore.load() {
        case .absent: return ["outcome": "signed_out"]
        case .error(let status): return failure(keychainMessage(status))
        case .found(let session): return sessionSummary(session)
        }
    }

    static func accessToken() -> [String: Any] {
        lock.lock()
        defer { lock.unlock() }
        let stored: StoredSession
        switch SessionStore.load() {
        case .absent: return ["outcome": "signed_out"]
        case .error(let status): return failure(keychainMessage(status))
        case .found(let session): stored = session
        }
        let now = Int64(Date().timeIntervalSince1970)
        if stored.expiresAt - now >= AccountConfig.refreshMarginSeconds {
            return tokenReply(stored)
        }
        guard let anonKey = AnonKey.read() else { return failure(AnonKey.missingMessage) }
        guard let reply = BoundedRequest.post(
            path: "/auth/v1/token", query: "grant_type=refresh_token",
            json: ["refresh_token": stored.refreshToken], anonKey: anonKey
        ) else {
            return failure("The sign-in service could not be reached to refresh the session.")
        }
        if reply.status == 400 || reply.status == 401 || reply.status == 403 {
            SessionStore.clear()
            return ["outcome": "signed_out", "message": "The session is no longer valid; sign in again."]
        }
        guard reply.status == 200,
              let refreshed = TokenParsing.session(from: reply.body, now: Date(), fallbackProvider: stored.provider)
        else {
            return failure("The session refresh failed (HTTP \(reply.status)).")
        }
        guard refreshed.userId == stored.userId else {
            SessionStore.clear()
            return ["outcome": "signed_out", "message": "The refreshed session belongs to a different account; signed out."]
        }
        let status = SessionStore.save(refreshed)
        guard status == errSecSuccess else { return failure(keychainMessage(status)) }
        return tokenReply(refreshed)
    }

    private static func tokenReply(_ session: StoredSession) -> [String: Any] {
        ["outcome": "signed_in", "accessToken": session.accessToken, "expiresAt": session.expiresAt]
    }

    // MARK: sign out

    static func signOut() -> [String: Any] {
        lock.lock()
        let stored: StoredSession?
        if case .found(let session) = SessionStore.load() { stored = session } else { stored = nil }
        let status = SessionStore.clear()
        lock.unlock()
        guard status == errSecSuccess else { return failure(keychainMessage(status)) }
        // Best-effort server revocation; local state is already gone either way.
        if let stored, let anonKey = AnonKey.read() {
            _ = BoundedRequest.post(
                path: "/auth/v1/logout", query: "scope=local", json: [:],
                anonKey: anonKey, bearer: stored.accessToken
            )
        }
        return ["outcome": "signed_out", "message": "Signed out."]
    }
}

// MARK: - Device key

enum DeviceKey {
    enum Storage: String {
        case secureEnclave = "secure_enclave"
        case keychain = "keychain"
    }

    private static let lock = NSLock()

    private static var lookup: [String: Any] {
        [
            kSecClass as String: kSecClassKey,
            kSecAttrApplicationTag as String: AccountConfig.deviceKeyTag,
            kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
            kSecAttrKeyClass as String: kSecAttrKeyClassPrivate,
            kSecUseDataProtectionKeychain as String: true,
        ]
    }

    static func storage(of key: SecKey) -> Storage {
        let attributes = SecKeyCopyAttributes(key) as? [String: Any]
        let token = attributes?[kSecAttrTokenID as String] as? String
        return token == (kSecAttrTokenIDSecureEnclave as String) ? .secureEnclave : .keychain
    }

    /// Loads the key, creating it once. The Secure Enclave is preferred; the fallback is a
    /// non-exportable Keychain key, and the caller learns which one it got.
    static func loadOrCreate() -> Result<(key: SecKey, storage: Storage), KeyError> {
        lock.lock()
        defer { lock.unlock() }
        var query = lookup
        query[kSecReturnRef as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let found = SecItemCopyMatching(query as CFDictionary, &item)
        if found == errSecSuccess, let item, CFGetTypeID(item) == SecKeyGetTypeID() {
            let key = item as! SecKey  // swiftlint:disable:this force_cast
            return .success((key, storage(of: key)))
        }
        if found != errSecItemNotFound { return .failure(KeyError(status: found)) }

        var lastStatus = errSecInternalError
        for enclave in [true, false] {
            switch create(secureEnclave: enclave) {
            case .success(let key): return .success((key, enclave ? .secureEnclave : .keychain))
            case .failure(let error): lastStatus = error.status
            }
        }
        return .failure(KeyError(status: lastStatus))
    }

    struct KeyError: Error {
        let status: OSStatus
    }

    private static func create(secureEnclave: Bool) -> Result<SecKey, KeyError> {
        var privateAttributes: [String: Any] = [
            kSecAttrIsPermanent as String: true,
            kSecAttrApplicationTag as String: AccountConfig.deviceKeyTag,
        ]
        var attributes: [String: Any] = [
            kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
            kSecAttrKeySizeInBits as String: 256,
            kSecUseDataProtectionKeychain as String: true,
        ]
        if secureEnclave {
            var accessError: Unmanaged<CFError>?
            guard let access = SecAccessControlCreateWithFlags(
                nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, .privateKeyUsage, &accessError
            ) else { return .failure(KeyError(status: errSecParam)) }
            privateAttributes[kSecAttrAccessControl as String] = access
            attributes[kSecAttrTokenID as String] = kSecAttrTokenIDSecureEnclave
        } else {
            privateAttributes[kSecAttrIsExtractable as String] = false
            privateAttributes[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        }
        attributes[kSecPrivateKeyAttrs as String] = privateAttributes
        var error: Unmanaged<CFError>?
        guard let key = SecKeyCreateRandomKey(attributes as CFDictionary, &error) else {
            let code = (error?.takeRetainedValue() as Error?).map { ($0 as NSError).code } ?? Int(errSecInternalError)
            return .failure(KeyError(status: OSStatus(code)))
        }
        return .success(key)
    }

    /// ECDSA over SHA-256 of the raw message, X9.62 DER.
    static func sign(_ message: Data, with key: SecKey) -> Data? {
        var error: Unmanaged<CFError>?
        return SecKeyCreateSignature(key, .ecdsaSignatureMessageX962SHA256, message as CFData, &error) as Data?
    }

    static func publicKeyX963(of key: SecKey) -> Data? {
        guard let publicKey = SecKeyCopyPublicKey(key) else { return nil }
        var error: Unmanaged<CFError>?
        return SecKeyCopyExternalRepresentation(publicKey, &error) as Data?
    }

    static func signChallenge(base64URL challenge: String) -> [String: Any] {
        guard let raw = AccountCodec.decodeBase64URL(challenge), raw.count >= 16, raw.count <= 1024 else {
            return failure("The challenge must be 16 to 1024 bytes of base64url.")
        }
        let key: SecKey
        let storage: Storage
        switch loadOrCreate() {
        case .failure(let error): return failure(keychainMessage(error.status))
        case .success(let loaded): (key, storage) = (loaded.key, loaded.storage)
        }
        guard let publicKey = publicKeyX963(of: key), let signature = sign(raw, with: key) else {
            return failure("The device key could not sign the challenge.")
        }
        return [
            "outcome": "ok",
            "devicePublicKeyB64Url": AccountCodec.base64URL(publicKey),
            "signatureB64Url": AccountCodec.base64URL(signature),
            "thumbprint": AccountCodec.thumbprint(publicKeyX963: publicKey),
            "keyStorage": storage.rawValue,
        ]
    }
}

// MARK: - Integrity

enum AccountIntegrity {
    static let requirement =
        "anchor apple generic and identifier \"\(AccountConfig.bundleIdentifier)\" "
        + "and certificate leaf[subject.OU] = \"\(AccountConfig.teamIdentifier)\""

    private static let lock = NSLock()
    private static var cachedResult: [String: Any]?

    static func check() -> [String: Any] {
        lock.lock()
        defer { lock.unlock() }
        if let cachedResult { return cachedResult }
        let result = evaluate()
        cachedResult = result
        return result
    }

    private static func outcome(_ name: String, _ detail: String) -> [String: Any] {
        ["outcome": name, "detail": detail]
    }

    private static func evaluate() -> [String: Any] {
        #if !DEBUG
        denyDebuggerAttachment()
        if isBeingDebugged() { return outcome("tampered", "A debugger is attached.") }
        #endif

        var selfCode: SecCode?
        guard SecCodeCopySelf([], &selfCode) == errSecSuccess, let code = selfCode else {
            return outcome("unknown", "The running code object could not be obtained.")
        }
        var requirementRef: SecRequirement?
        guard SecRequirementCreateWithString(requirement as CFString, [], &requirementRef) == errSecSuccess,
              let requirementRef
        else {
            return outcome("unknown", "The pinned requirement could not be compiled.")
        }
        let dynamicStatus = SecCodeCheckValidity(code, SecCSFlags(rawValue: kSecCSStrictValidate), requirementRef)
        if dynamicStatus != errSecSuccess { return classify(dynamicStatus, stage: "running code") }

        var staticCode: SecStaticCode?
        guard SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode else {
            return outcome("unknown", "The static code object could not be obtained.")
        }
        let flags = SecCSFlags(rawValue: kSecCSCheckAllArchitectures | kSecCSStrictValidate | kSecCSCheckNestedCode)
        let staticStatus = SecStaticCodeCheckValidity(staticCode, flags, requirementRef)
        if staticStatus != errSecSuccess { return classify(staticStatus, stage: "bundle on disk") }
        return outcome("ok", "Signature valid and pinned to team \(AccountConfig.teamIdentifier).")
    }

    /// An unsigned binary proves nothing either way (development), so it is unknown; a signature
    /// that exists but does not verify or does not satisfy the pin is tampering.
    private static func classify(_ status: OSStatus, stage: String) -> [String: Any] {
        if status == errSecCSUnsigned {
            return outcome("unknown", "The \(stage) is not code signed.")
        }
        return outcome("tampered", "The \(stage) failed signature or requirement validation (OSStatus \(status)).")
    }

    private static func denyDebuggerAttachment() {
        typealias PtraceFn = @convention(c) (CInt, pid_t, UnsafeMutableRawPointer?, CInt) -> CInt
        let rtldDefault = UnsafeMutableRawPointer(bitPattern: -2)
        if let symbol = dlsym(rtldDefault, "ptrace") {
            let ptraceFn = unsafeBitCast(symbol, to: PtraceFn.self)
            _ = ptraceFn(31 /* PT_DENY_ATTACH */, 0, nil, 0)
        }
    }

    /// Fail-open on a sysctl error: only a positive P_TRACED is evidence.
    private static func isBeingDebugged() -> Bool {
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.stride
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()]
        guard sysctl(&mib, UInt32(mib.count), &info, &size, nil, 0) == 0 else { return false }
        return (info.kp_proc.p_flag & P_TRACED) != 0
    }
}

// MARK: - C ABI

@_cdecl("onrecord_account_sign_in")
public func onrecordAccountSignIn(_ provider: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    guard let provider else { return respond(failure("No provider was supplied.")) }
    return respond(AccountService.signIn(providerName: String(cString: provider)))
}

@_cdecl("onrecord_account_session")
public func onrecordAccountSession() -> UnsafeMutablePointer<CChar>? {
    respond(AccountService.currentSession())
}

@_cdecl("onrecord_account_access_token")
public func onrecordAccountAccessToken() -> UnsafeMutablePointer<CChar>? {
    respond(AccountService.accessToken())
}

@_cdecl("onrecord_account_sign_out")
public func onrecordAccountSignOut() -> UnsafeMutablePointer<CChar>? {
    respond(AccountService.signOut())
}

@_cdecl("onrecord_account_device_sign")
public func onrecordAccountDeviceSign(_ challenge: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    guard let challenge else { return respond(failure("No challenge was supplied.")) }
    return respond(DeviceKey.signChallenge(base64URL: String(cString: challenge)))
}

@_cdecl("onrecord_account_integrity_check")
public func onrecordAccountIntegrityCheck() -> UnsafeMutablePointer<CChar>? {
    respond(AccountIntegrity.check())
}

@_cdecl("onrecord_account_secure_get")
public func onrecordAccountSecureGet(_ key: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    guard let key else { return respond(failure("No key was supplied.")) }
    return respond(SecureItems.get(String(cString: key)))
}

@_cdecl("onrecord_account_secure_set")
public func onrecordAccountSecureSet(
    _ key: UnsafePointer<CChar>?, _ value: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>? {
    guard let key, let value else { return respond(failure("No key or value was supplied.")) }
    return respond(SecureItems.set(String(cString: key), String(cString: value)))
}

@_cdecl("onrecord_account_free_string")
public func onrecordAccountFreeString(_ value: UnsafeMutablePointer<CChar>?) {
    free(value)
}
