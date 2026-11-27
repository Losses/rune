package ci.not.rune.applemusic

import android.content.Context
import com.apple.android.sdk.authentication.TokenProvider

/**
 * The tokens MusicKit plays with: the developer token of [DeveloperTokens], and the user's
 * Music User Token with the storefronts that came with it, kept in the app's private
 * preferences. None of them is ever logged.
 */
internal class TokenStore(
    context: Context,
    private val developerTokens: DeveloperTokens,
) : TokenProvider {
    private companion object {
        const val USER_TOKEN = "userToken"
        const val STOREFRONT = "storefront"
        const val MUSIC_STOREFRONT = "appleMusicStorefront"
    }

    private val preferences = context.getSharedPreferences("apple_music", Context.MODE_PRIVATE)

    override fun getDeveloperToken(): String = developerTokens.current

    override fun getUserToken(): String? = preferences.getString(USER_TOKEN, null)

    override fun getAppleMusicStoreFrontId(): String? =
        preferences.getString(MUSIC_STOREFRONT, null)

    /** Keeps what a sign-in gave. */
    fun store(userToken: String, storefront: String?, musicStorefront: String?) {
        preferences.edit()
            .putString(USER_TOKEN, userToken)
            .putString(STOREFRONT, storefront)
            .putString(MUSIC_STOREFRONT, musicStorefront)
            .apply()
    }

    /** Forgets the user's token, which Apple Music no longer accepts. */
    fun forget() {
        preferences.edit().clear().apply()
    }
}
