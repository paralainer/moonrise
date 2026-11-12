// The resume rule's shared cases (api/resume-rules.json at the repository's root, copied into the
// test bundle by project.yml): the Android apps (clients/android/core/src/test/.../ResumeRulesCasesTest.kt)
// and the Windows app (clients/windows/Moonrise.Core.Tests/SharedCasesTests.cs) run the same ones.
import Foundation
import Testing
@testable import Moonrise

private final class ResumeCasesBundleToken {}

@Suite struct ResumeRulesCasesTests {
    struct File: Decodable {
        struct User: Decodable { let position: Double?; let played: Bool? }
        struct Case: Decodable { let name: String; let user: User?; let expected: Double? }
        let cases: [Case]
    }

    @Test func theSharedCases() throws {
        let url = try #require(Bundle(for: ResumeCasesBundleToken.self).url(forResource: "resume-rules", withExtension: "json"),
                               "api/resume-rules.json isn't in the test bundle (project.yml)")
        let file = try JSONDecoder().decode(File.self, from: Data(contentsOf: url))
        #expect(file.cases.count >= 10)
        for c in file.cases {
            let item = Fixture.movie("m", user: c.user.map { UserState(position: $0.position, played: $0.played) })
            #expect(item.resumeSeconds == c.expected, Comment(rawValue: c.name))
        }
    }
}
