package id.neptunus.desktop

import android.Manifest
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.os.Build
import android.os.Bundle
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // the local library reads audio and video files straight from the shared folders
    val read = if (Build.VERSION.SDK_INT >= 33) arrayOf(Manifest.permission.READ_MEDIA_AUDIO, Manifest.permission.READ_MEDIA_VIDEO)
      else arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
    val missing = read.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
    if (missing.isNotEmpty()) requestPermissions(missing.toTypedArray(), 0)
  }

  override fun onWebViewCreate(webView: WebView) {
    MediaService.attach(webView)
  }

  override fun onDestroy() {
    MediaService.detach()
    super.onDestroy()
  }

  // uiMode is in the manifest's configChanges, so switching dark mode doesn't recreate the activity:
  // pick the status bar icon colour again, or they stay dark on the dark app bar
  override fun onConfigurationChanged(newConfig: Configuration) {
    super.onConfigurationChanged(newConfig)
    enableEdgeToEdge()
  }
}
