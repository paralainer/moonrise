// The network layer (`HTTP`): a request carries only what it is given, so the server's token goes
// only where a caller passes it, and the defaults stay URLRequest's own (the session's
// configuration then applies as it did before the layer); and each session keeps its settings.
import Foundation
import Testing
@testable import Moonrise

struct HTTPTests {
    private let url = URL(string: "http://192.168.1.10:8097/v1/items/x")!

    @Test func plainRequestIsURLRequestsDefault() {
        let req = HTTP.request(url)
        let plain = URLRequest(url: url)
        #expect(req == plain)
        #expect(req.httpMethod == "GET")
        #expect(req.allHTTPHeaderFields?.isEmpty ?? true)
        #expect(req.timeoutInterval == 60)
        #expect(req.cachePolicy == .useProtocolCachePolicy)
    }

    @Test func givenFieldsAreSet() {
        let req = HTTP.request(url, method: "DELETE", headers: ["Authorization": "Bearer t", "Range": "bytes=0-1"], timeout: 4,
                               cachePolicy: .reloadIgnoringLocalCacheData)
        #expect(req.httpMethod == "DELETE")
        #expect(req.timeoutInterval == 4)
        #expect(req.cachePolicy == .reloadIgnoringLocalCacheData)
        #expect(req.value(forHTTPHeaderField: "Authorization") == "Bearer t")
        #expect(req.value(forHTTPHeaderField: "Range") == "bytes=0-1")
        #expect(req.allHTTPHeaderFields?.count == 2)
    }

    @Test func sameAsHandBuilt() {
        var hand = URLRequest(url: url, timeoutInterval: 10)
        hand.cachePolicy = .reloadIgnoringLocalCacheData
        hand.setValue("Bearer t", forHTTPHeaderField: "Authorization")
        #expect(HTTP.request(url, headers: ["Authorization": "Bearer t"], timeout: 10, cachePolicy: .reloadIgnoringLocalCacheData) == hand)
    }

    @Test func noTokenWithoutServerHeaders() {
        let client = MoonriseClient(sourceId: nil, baseURL: URL(string: "http://192.168.1.10:8097")!, token: "fake-token")
        #expect(client.headers == ["Authorization": "Bearer fake-token"])
        // A request to another host is built without them.
        #expect(HTTP.request(URL(string: "https://example.org/a.mp4")!).value(forHTTPHeaderField: "Authorization") == nil)
    }

    @Test func status() {
        #expect(HTTP.status(HTTPURLResponse(url: url, statusCode: 410, httpVersion: nil, headerFields: nil)) == 410)
        #expect(HTTP.status(URLResponse(url: url, mimeType: nil, expectedContentLength: 0, textEncodingName: nil)) == 0)
        #expect(HTTP.status(nil) == 0)
    }

    @Test func sessionSettings() {
        let api = HTTP.api.configuration
        #expect(api.timeoutIntervalForRequest == 20)
        #expect(api.urlCache == nil)

        let art = HTTP.artworkSession().configuration
        #expect(art.timeoutIntervalForRequest == 30)
        #expect(art.httpMaximumConnectionsPerHost == 6)
        #expect(art.urlCache == nil)

        let set = HTTP.downloadSetSession(cellular: false).configuration
        #expect(set.timeoutIntervalForRequest == 90)
        #expect(set.httpMaximumConnectionsPerHost == 6)
        #expect(set.allowsCellularAccess == false)
        #expect(set.requestCachePolicy == .reloadIgnoringLocalCacheData)
        #expect(set.urlCache == nil)
        #expect(HTTP.downloadSetSession(cellular: true).configuration.allowsCellularAccess)

        let sweep = HTTP.sweep.configuration
        #expect(sweep.timeoutIntervalForRequest == 1.5)
        #expect(sweep.timeoutIntervalForResource == 2)
        #expect(sweep.waitsForConnectivity == false)
        #expect(sweep.httpMaximumConnectionsPerHost == 1)
    }
}
