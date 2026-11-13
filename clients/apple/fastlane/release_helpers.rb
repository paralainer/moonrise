# Plain-Ruby rules of the Fastfile's lanes, kept apart so they run without fastlane or App Store
# Connect: test/release_helpers_test.rb (scripts/tests/fastlane-helpers.test.sh runs it).
module MoonriseRelease
  module_function

  # Whether the submit lane was told to go: confirm:yes on the command line, which fastlane turns
  # into true before the lane sees it. Anything else ("no", "true", 1, nil) is a no.
  def submit_confirmed?(value)
    [true, "yes"].include?(value)
  end

  # Sets an App Store version to manual release (releaseType MANUAL, so an approved version waits
  # for someone to release it) through the App Store Connect API, then reads it back. `client`
  # answers patch(path, body) and get(path) like Spaceship's API clients. Raises when the version
  # doesn't read back as MANUAL.
  def manual_release!(client, version_id)
    path = "v1/appStoreVersions/#{version_id}"
    client.patch(path, { data: { type: "appStoreVersions", id: version_id, attributes: { releaseType: "MANUAL" } } })
    got = release_type(client, version_id)
    raise "version #{version_id}: releaseType reads back #{got.inspect}, not MANUAL" unless got == "MANUAL"
    got
  end

  # The version's releaseType, read from the API (MANUAL, AFTER_APPROVAL or SCHEDULED).
  def release_type(client, version_id)
    body = client.get("v1/appStoreVersions/#{version_id}").body
    body.dig("data", "attributes", "releaseType")
  end
end
