package app.moonrise.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Ratings beside TMDB's on the title page. */
class RatingsTest {
    @Test
    fun decodesTheServersRatings() {
        val item = MoonriseApi.json.decodeFromString(Item.serializer(),
            """{"id":"m1","kind":"movie","community_rating":7.9,
               "ratings":{"imdb":{"value":8.1,"votes":123456},"rotten_tomatoes":{"value":94},"metacritic":{"value":78},"letterboxd":{"value":4.1}}}""")
        assertEquals(8.1, item.ratings?.imdb?.value)
        assertEquals(123456L, item.ratings?.imdb?.votes)
        assertEquals(94.0, item.ratings?.rottenTomatoes?.value)
        assertEquals("★ 7.9   IMDb 8.1   RT 94%   MC 78", ratingLine(item.communityRating, item.ratings))
        // An older server: no field, the line as before.
        val old = MoonriseApi.json.decodeFromString(Item.serializer(), """{"id":"m1","kind":"movie","community_rating":7.9}""")
        assertNull(old.ratings)
        assertEquals("★ 7.9", ratingLine(old.communityRating, old.ratings))
    }

    @Test
    fun missingSourcesAreLeftOut() {
        assertEquals("IMDb 6.0   MC 55", ratingLine(null, Ratings(imdb = Score(6.0), metacritic = Score(55.0))))
        assertNull(ratingLine(null, Ratings()))
        assertNull(ratingLine(null, null))
    }
}
