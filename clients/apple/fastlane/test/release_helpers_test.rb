# Tests of clients/apple/fastlane/release_helpers.rb, plain Ruby (no gems): ruby release_helpers_test.rb.
# The App Store Connect API is a stub that keeps what it is sent.
require_relative "../release_helpers"

$failed = 0
$passed = 0
def check(ok, what)
  if ok
    $passed += 1
  else
    $failed += 1
    puts "FAIL release_helpers_test.rb: #{what}"
  end
end

# submit's guard: true (fastlane's confirm:yes) or "yes" go; nothing else does.
check(MoonriseRelease.submit_confirmed?(true), "confirm:yes as fastlane passes it (true) is accepted")
check(MoonriseRelease.submit_confirmed?("yes"), "\"yes\" is accepted")
[nil, false, "", "no", "true", "YES", "y", 1, "1", :yes].each do |v|
  check(!MoonriseRelease.submit_confirmed?(v), "confirm #{v.inspect} is refused")
end

# A stub client: patch stores the release type when `takes` is true, get answers what is stored.
class StubClient
  Response = Struct.new(:body)
  attr_reader :patches
  def initialize(stored, takes: true)
    @stored = stored
    @takes = takes
    @patches = []
  end

  def patch(path, body)
    @patches << [path, body]
    @stored = body.dig(:data, :attributes, :releaseType) if @takes
    Response.new({})
  end

  def get(_path)
    Response.new({ "data" => { "id" => "v1", "attributes" => { "releaseType" => @stored } } })
  end
end

c = StubClient.new("AFTER_APPROVAL")
check(MoonriseRelease.manual_release!(c, "v1") == "MANUAL", "manual_release! returns MANUAL once set")
path, body = c.patches.first
check(path == "v1/appStoreVersions/v1", "patches the version (#{path.inspect})")
check(body == { data: { type: "appStoreVersions", id: "v1", attributes: { releaseType: "MANUAL" } } },
      "sends releaseType MANUAL (#{body.inspect})")

# The API answers but the version stays on automatic release: an error, not a quiet pass.
c = StubClient.new("AFTER_APPROVAL", takes: false)
begin
  MoonriseRelease.manual_release!(c, "v1")
  check(false, "a version still AFTER_APPROVAL after the patch raises")
rescue RuntimeError => e
  check(e.message.include?("AFTER_APPROVAL"), "the error names what it read back (#{e.message})")
end
check(MoonriseRelease.release_type(StubClient.new("SCHEDULED"), "v1") == "SCHEDULED", "release_type reads the API's value")

# The Fastfile uses the helpers in the submit lane (guard, manual release before and after deliver).
fastfile = File.read(File.join(__dir__, "..", "Fastfile"))
submit = fastfile[/lane :submit do.*?\n  end\n/m].to_s
check(submit.include?("MoonriseRelease.submit_confirmed?(options[:confirm])"), "the submit lane's guard is submit_confirmed?")
before = submit.index("MoonriseRelease.manual_release!")
deliver = submit.index("deliver(")
check(before && deliver && before < deliver, "the submit lane sets manual release before deliver submits")

puts "release_helpers_test.rb: #{$passed} passed, #{$failed} failed"
exit($failed.zero? ? 0 : 1)
