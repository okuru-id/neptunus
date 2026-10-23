package id.neptunus.desktop

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.core.content.FileProvider
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

// Keeps the app alive while music plays in the background and shows the playing song with
// controls on the lock screen and in the notification shade. The UI reports the song through
// `window.NeptunusAndroid`; the controls go back to it as `window.__mediaKey` / `__mediaSeek`.
class MediaService : Service() {
  companion object {
    private const val CHANNEL = "playback"
    private var instance: MediaService? = null
    private var webView: WebView? = null
    private var title = ""
    private var artist = ""
    private var album = ""
    private var duration = 0.0
    private var state = -1 // -1 stopped, 0 paused, 1 playing
    private var position = 0.0

    fun attach(view: WebView) {
      webView = view
      view.addJavascriptInterface(Bridge(view), "NeptunusAndroid")
    }

    fun detach() {
      webView = null
    }

    private fun js(code: String) {
      webView?.evaluateJavascript(code, null)
    }

    private fun changed(ctx: Context) {
      val s = instance
      when {
        s != null && state < 0 -> s.stopSelf()
        s != null -> s.refresh()
        state == 1 -> ctx.startForegroundService(Intent(ctx, MediaService::class.java))
      }
    }
  }

  // Called on the WebView's JavaBridge thread; only the app's own page may drive it, not a site
  // opened in the same webview (extension verification).
  class Bridge(private val view: WebView) {
    private val main = Handler(Looper.getMainLooper())

    private fun ours(block: () -> Unit) = main.post {
      if (view.url?.startsWith("http://tauri.localhost") == true) block()
    }

    @JavascriptInterface
    fun meta(t: String, a: String, al: String, secs: Double) = ours {
      title = t; artist = a; album = al; duration = secs; position = 0.0
      changed(view.context)
    }

    @JavascriptInterface
    fun state(s: Int, secs: Double) = ours {
      state = s; position = secs
      changed(view.context)
    }

    // an update downloaded into the cache (`android_apk`) goes to the system installer; without the
    // "install unknown apps" permission the settings page for it opens instead and this returns false
    @JavascriptInterface
    fun installApk(path: String): Boolean {
      val ctx = view.context
      val allowed = Build.VERSION.SDK_INT < 26 || ctx.packageManager.canRequestPackageInstalls()
      ours {
        val file = java.io.File(path).canonicalFile
        if (!file.path.startsWith(ctx.cacheDir.canonicalPath + "/") || !file.name.endsWith(".apk")) return@ours
        val intent = if (allowed) {
          Intent(Intent.ACTION_VIEW)
            .setDataAndType(FileProvider.getUriForFile(ctx, ctx.packageName + ".fileprovider", file), "application/vnd.android.package-archive")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        } else Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + ctx.packageName))
        ctx.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
      }
      return allowed
    }

    // the video player's fullscreen: the WebView's own requestFullscreen does nothing here, so hide
    // the system bars and turn to landscape ourselves
    @JavascriptInterface
    fun fullscreen(on: Boolean) = ours {
      val a = view.context as? Activity ?: return@ours
      val bars = WindowCompat.getInsetsController(a.window, a.window.decorView)
      if (on) {
        bars.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        bars.hide(WindowInsetsCompat.Type.systemBars())
      } else bars.show(WindowInsetsCompat.Type.systemBars())
      a.requestedOrientation = if (on) ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE else ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
    }
  }

  private lateinit var session: MediaSession

  override fun onBind(intent: Intent?) = null

  override fun onCreate() {
    super.onCreate()
    instance = this
    getSystemService(NotificationManager::class.java).createNotificationChannel(
      NotificationChannel(CHANNEL, "Pemutaran", NotificationManager.IMPORTANCE_LOW)
    )
    session = MediaSession(this, "Neptunus").apply {
      setCallback(object : MediaSession.Callback() {
        override fun onPlay() = js("window.__mediaKey?.('play')")
        override fun onPause() = js("window.__mediaKey?.('pause')")
        override fun onSkipToNext() = js("window.__mediaKey?.('next')")
        override fun onSkipToPrevious() = js("window.__mediaKey?.('prev')")
        override fun onSeekTo(pos: Long) = js("window.__mediaSeek?.(${pos / 1000.0})")
      })
      isActive = true
    }
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    intent?.action?.let { js("window.__mediaKey?.('$it')") }
    refresh()
    return START_NOT_STICKY
  }

  override fun onDestroy() {
    instance = null
    session.release()
    super.onDestroy()
  }

  private fun action(icon: Int, label: String, key: String) = Notification.Action.Builder(
    Icon.createWithResource(this, icon), label,
    PendingIntent.getService(this, key.hashCode(), Intent(this, MediaService::class.java).setAction(key), PendingIntent.FLAG_IMMUTABLE)
  ).build()

  fun refresh() {
    val playing = state == 1
    session.setMetadata(
      MediaMetadata.Builder()
        .putString(MediaMetadata.METADATA_KEY_TITLE, title)
        .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
        .putString(MediaMetadata.METADATA_KEY_ALBUM, album)
        .putLong(MediaMetadata.METADATA_KEY_DURATION, (duration * 1000).toLong())
        .build()
    )
    session.setPlaybackState(
      PlaybackState.Builder()
        .setActions(
          PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or PlaybackState.ACTION_PLAY_PAUSE or
            PlaybackState.ACTION_SKIP_TO_NEXT or PlaybackState.ACTION_SKIP_TO_PREVIOUS or PlaybackState.ACTION_SEEK_TO
        )
        .setState(if (playing) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED, (position * 1000).toLong(), if (playing) 1f else 0f)
        .build()
    )
    val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
    val notification = Notification.Builder(this, CHANNEL)
      .setSmallIcon(android.R.drawable.ic_media_play)
      .setContentTitle(title)
      .setContentText(artist)
      .setContentIntent(open)
      .setVisibility(Notification.VISIBILITY_PUBLIC)
      .setOngoing(playing)
      .addAction(action(android.R.drawable.ic_media_previous, "Sebelumnya", "prev"))
      .addAction(
        if (playing) action(android.R.drawable.ic_media_pause, "Jeda", "pause")
        else action(android.R.drawable.ic_media_play, "Putar", "play")
      )
      .addAction(action(android.R.drawable.ic_media_next, "Berikutnya", "next"))
      .setStyle(Notification.MediaStyle().setMediaSession(session.sessionToken).setShowActionsInCompactView(0, 1, 2))
      .build()
    if (Build.VERSION.SDK_INT >= 29) startForeground(1, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
    else startForeground(1, notification)
  }
}
