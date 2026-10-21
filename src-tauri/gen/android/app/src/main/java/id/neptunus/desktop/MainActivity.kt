package id.neptunus.desktop

import android.Manifest
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.os.Build
import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // the local library reads audio files straight from the shared Music folder
    val read = if (Build.VERSION.SDK_INT >= 33) Manifest.permission.READ_MEDIA_AUDIO else Manifest.permission.READ_EXTERNAL_STORAGE
    if (checkSelfPermission(read) != PackageManager.PERMISSION_GRANTED) requestPermissions(arrayOf(read), 0)
  }

  // uiMode is in the manifest's configChanges, so switching dark mode doesn't recreate the activity:
  // pick the status bar icon colour again, or they stay dark on the dark app bar
  override fun onConfigurationChanged(newConfig: Configuration) {
    super.onConfigurationChanged(newConfig)
    enableEdgeToEdge()
  }
}
