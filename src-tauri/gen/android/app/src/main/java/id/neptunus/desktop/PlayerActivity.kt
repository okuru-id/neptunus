package id.neptunus.desktop

import android.content.Context
import android.net.Uri
import android.os.Bundle
import android.os.Looper
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.MimeTypes
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.Tracks
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.Renderer
import androidx.media3.exoplayer.text.TextOutput
import androidx.media3.exoplayer.text.TextRenderer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.ui.PlayerView
import org.json.JSONObject

// Films on Android play here rather than in the WebView's <video>: ExoPlayer reads MKV, HEVC and every
// audio codec the phone can decode, with its own controls for subtitles and audio tracks. Started by
// `NeptunusAndroid.playNative` with `{url, title, headers, subs: [{url, name, lang}], subOn, position}`
// (ms); on closing it tells the page where it got to (`window.__nativeVideoClosed`).
class PlayerActivity : ComponentActivity() {
  private lateinit var player: ExoPlayer
  private var error: String? = null
  private var reported = false

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val o = JSONObject(intent.getStringExtra("json") ?: "{}")
    val headers = o.optJSONObject("headers")?.let { h -> h.keys().asSequence().associateWith { h.getString(it) } } ?: emptyMap()
    val http = DefaultHttpDataSource.Factory()
      .setAllowCrossProtocolRedirects(true)
      .setUserAgent(headers.entries.firstOrNull { it.key.equals("user-agent", true) }?.value ?: "Mozilla/5.0")
      .setDefaultRequestProperties(headers.filterKeys { !it.equals("user-agent", true) })
    // Subtitles left unparsed by the source (below) reach the text renderer as raw text/vtt, srt or ssa,
    // which Media3 only decodes there with its legacy decoding on; off, every film with a subtitle fails
    // ("Legacy decoding is disabled, can't handle text/vtt samples").
    val renderers = object : DefaultRenderersFactory(this) {
      override fun buildTextRenderers(context: Context, output: TextOutput, outputLooper: Looper, extensionRendererMode: Int, out: ArrayList<Renderer>) {
        super.buildTextRenderers(context, output, outputLooper, extensionRendererMode, out)
        out.filterIsInstance<TextRenderer>().forEach { it.experimentalSetLegacyDecodingEnabled(true) }
      }
    }
    player = ExoPlayer.Builder(this, renderers)
      // the film starts without waiting for every subtitle file to be read
      .setMediaSourceFactory(DefaultMediaSourceFactory(DefaultDataSource.Factory(this, http)).experimentalParseSubtitlesDuringExtraction(false))
      .setAudioAttributes(AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MOVIE).build(), true)
      .build()

    // the extensions' subtitles, the one picked in the app (the first, with subOn) shown from the start
    val subs = o.optJSONArray("subs")
    val subOn = o.optBoolean("subOn")
    val configs = (0 until (subs?.length() ?: 0)).map { i ->
      val s = subs!!.getJSONObject(i)
      val url = s.getString("url")
      MediaItem.SubtitleConfiguration.Builder(Uri.parse(url))
        .setMimeType(subtitleType(url))
        .setLanguage(s.optString("lang").ifEmpty { null })
        .setLabel(s.optString("name").ifEmpty { null })
        .setSelectionFlags(if (i == 0 && subOn) C.SELECTION_FLAG_DEFAULT else 0)
        .build()
    }
    if (subOn) configs.firstOrNull()?.language?.let {
      player.trackSelectionParameters = player.trackSelectionParameters.buildUpon().setPreferredTextLanguage(it).build()
    }
    val item = MediaItem.Builder()
      .setUri(Uri.parse(o.getString("url")))
      .setMediaMetadata(MediaMetadata.Builder().setTitle(o.optString("title")).build())
      .setSubtitleConfigurations(configs)
      .build()
    player.setMediaItem(item, o.optLong("position"))
    player.addListener(object : Player.Listener {
      override fun onPlayerError(e: PlaybackException) {
        error = listOfNotNull(e.errorCodeName, e.cause?.message).joinToString(": ")
        finish()
      }

      override fun onPlaybackStateChanged(state: Int) {
        if (state == Player.STATE_ENDED) finish()
      }

      // sound this phone can't decode (DTS, often AC3): the picture plays on in silence, so say so
      override fun onTracksChanged(tracks: Tracks) {
        val audio = tracks.groups.filter { it.type == C.TRACK_TYPE_AUDIO }
        if (audio.isNotEmpty() && audio.none { it.isSupported }) {
          Toast.makeText(this@PlayerActivity, "Suara film ini tidak didukung HP ini. Coba putar di Pemutar lain (VLC).", Toast.LENGTH_LONG).show()
        }
      }
    })
    player.prepare()
    player.play()

    val view = PlayerView(this)
    view.player = player
    view.keepScreenOn = true
    view.setShowSubtitleButton(true)
    setContentView(view)
    // the whole screen: the bars come back with a swipe
    WindowCompat.getInsetsController(window, window.decorView).apply {
      systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
      hide(WindowInsetsCompat.Type.systemBars())
    }
  }

  override fun onPause() {
    super.onPause()
    player.pause()
  }

  override fun onDestroy() {
    report()
    player.release()
    super.onDestroy()
  }

  // once, as the player closes (or the system ends it): where it got to, in seconds
  private fun report() {
    if (reported) return
    reported = true
    val ended = player.playbackState == Player.STATE_ENDED
    val duration = player.duration.takeIf { it != C.TIME_UNSET }?.div(1000.0) ?: 0.0
    val json = JSONObject()
      .put("pos", player.currentPosition / 1000.0)
      .put("duration", duration)
      .put("eof", ended)
      .put("error", error ?: JSONObject.NULL)
    MediaService.videoClosed(json.toString())
  }

  // by the file's name; addon links without one are nearly always SRT
  private fun subtitleType(url: String): String {
    val name = Uri.parse(url).lastPathSegment.orEmpty().lowercase()
    return when {
      name.endsWith(".vtt") -> MimeTypes.TEXT_VTT
      name.endsWith(".ass") || name.endsWith(".ssa") -> MimeTypes.TEXT_SSA
      else -> MimeTypes.APPLICATION_SUBRIP
    }
  }
}
