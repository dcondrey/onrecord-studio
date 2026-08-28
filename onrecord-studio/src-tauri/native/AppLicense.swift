import Darwin
import Foundation
import StoreKit

private struct LicenseEnvelope: Encodable, Sendable {
    let outcome: String
    let message: String
    let bundleId: String?
    let appVersion: String?
    let originalAppVersion: String?
    let appTransactionId: String?
    let appId: UInt64?
    let environment: String?
    let originalPurchaseDate: String?
    let signedDate: String?
    let revocationDate: String?
}

private let expectedBundleIdentifier = "com.onrecord.studio"
private final class LicenseEnvelopeBox: @unchecked Sendable {
    private let lock = NSLock()
    private var value: LicenseEnvelope?

    func store(_ envelope: LicenseEnvelope) {
        lock.lock()
        value = envelope
        lock.unlock()
    }

    func load() -> LicenseEnvelope? {
        lock.lock()
        defer { lock.unlock() }
        return value
    }
}

private func verifiedPayload(_ transaction: AppTransaction) -> [String: Any]? {
    guard
        let object = try? JSONSerialization.jsonObject(with: transaction.jsonRepresentation)
            as? [String: Any]
    else {
        return nil
    }
    return object
}

private func iso8601String(_ date: Date) -> String {
    ISO8601DateFormatter().string(from: date)
}

private func dateFromMilliseconds(_ value: Any?) -> String? {
    guard let milliseconds = value as? NSNumber else { return nil }
    return iso8601String(Date(timeIntervalSince1970: milliseconds.doubleValue / 1_000))
}

private func verifiedEnvelope(_ transaction: AppTransaction) -> LicenseEnvelope {
    guard let payload = verifiedPayload(transaction) else {
        return LicenseEnvelope(
            outcome: "unverified",
            message: "StoreKit verified the transaction signature, but its signed payload could not be decoded.",
            bundleId: transaction.bundleID,
            appVersion: transaction.appVersion,
            originalAppVersion: transaction.originalAppVersion,
            appTransactionId: transaction.appTransactionID,
            appId: transaction.appID,
            environment: String(describing: transaction.environment),
            originalPurchaseDate: iso8601String(transaction.originalPurchaseDate),
            signedDate: iso8601String(transaction.signedDate),
            revocationDate: nil
        )
    }
    let revocationDate = dateFromMilliseconds(payload["revocationDate"])
    let bundleMatches = transaction.bundleID == expectedBundleIdentifier
    let outcome: String
    let message: String
    if !bundleMatches {
        outcome = "bundle_mismatch"
        message = "Apple verified a transaction for a different application bundle."
    } else if revocationDate != nil {
        outcome = "revoked"
        message = "Apple reports that this app purchase was revoked or refunded."
    } else {
        outcome = "verified"
        message = "Apple verified the paid app transaction for this installation."
    }
    return LicenseEnvelope(
        outcome: outcome,
        message: message,
        bundleId: transaction.bundleID,
        appVersion: transaction.appVersion,
        originalAppVersion: transaction.originalAppVersion,
        appTransactionId: transaction.appTransactionID,
        appId: transaction.appID,
        environment: String(describing: transaction.environment),
        originalPurchaseDate: iso8601String(transaction.originalPurchaseDate),
        signedDate: iso8601String(transaction.signedDate),
        revocationDate: revocationDate
    )
}

private func evaluate(refresh: Bool) async -> LicenseEnvelope {
    do {
        let result = try await (refresh ? AppTransaction.refresh() : AppTransaction.shared)
        switch result {
        case .verified(let transaction):
            return verifiedEnvelope(transaction)
        case .unverified(_, let error):
            return LicenseEnvelope(
                outcome: "unverified",
                message: "StoreKit could not verify the App Store transaction: \(error.localizedDescription)",
                bundleId: nil,
                appVersion: nil,
                originalAppVersion: nil,
                appTransactionId: nil,
                appId: nil,
                environment: nil,
                originalPurchaseDate: nil,
                signedDate: nil,
                revocationDate: nil
            )
        }
    } catch {
        return LicenseEnvelope(
            outcome: "unavailable",
            message: "The App Store purchase could not be loaded: \(error.localizedDescription)",
            bundleId: nil,
            appVersion: nil,
            originalAppVersion: nil,
            appTransactionId: nil,
            appId: nil,
            environment: nil,
            originalPurchaseDate: nil,
            signedDate: nil,
            revocationDate: nil
        )
    }
}

private func encodedEnvelope(refresh: Bool) -> UnsafeMutablePointer<CChar>? {
    let semaphore = DispatchSemaphore(value: 0)
    let box = LicenseEnvelopeBox()
    Task.detached(priority: .userInitiated) {
        box.store(await evaluate(refresh: refresh))
        semaphore.signal()
    }
    let completed: Bool
    if refresh {
        semaphore.wait()
        completed = true
    } else {
        completed = semaphore.wait(timeout: .now() + 20) == .success
    }
    let envelope = box.load() ?? LicenseEnvelope(
        outcome: "unavailable",
        message: completed
            ? "StoreKit verification completed without a readable result."
            : "StoreKit verification did not complete within 20 seconds.",
        bundleId: nil,
        appVersion: nil,
        originalAppVersion: nil,
        appTransactionId: nil,
        appId: nil,
        environment: nil,
        originalPurchaseDate: nil,
        signedDate: nil,
        revocationDate: nil
    )
    guard
        let data = try? JSONEncoder().encode(envelope),
        let json = String(data: data, encoding: .utf8)
    else {
        return strdup("{\"outcome\":\"unavailable\",\"message\":\"StoreKit returned an unreadable result.\"}")
    }
    return strdup(json)
}

@_cdecl("onrecord_storekit_app_transaction")
public func onrecordStoreKitAppTransaction(_ refresh: Bool) -> UnsafeMutablePointer<CChar>? {
    encodedEnvelope(refresh: refresh)
}

@_cdecl("onrecord_storekit_free_string")
public func onrecordStoreKitFreeString(_ value: UnsafeMutablePointer<CChar>?) {
    free(value)
}
