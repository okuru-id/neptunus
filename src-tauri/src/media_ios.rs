//! iOS lock screen / Control Center player (MPNowPlayingInfoCenter + MPRemoteCommandCenter).

use block2::RcBlock;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSDictionary, NSNumber, NSString};
use objc2_media_player::{
    MPChangePlaybackPositionCommandEvent, MPMediaItemPropertyAlbumTitle, MPMediaItemPropertyArtist,
    MPMediaItemPropertyPlaybackDuration, MPMediaItemPropertyTitle, MPNowPlayingInfoCenter,
    MPNowPlayingInfoPropertyElapsedPlaybackTime, MPNowPlayingInfoPropertyPlaybackRate, MPRemoteCommandCenter,
    MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
};
use std::ptr::NonNull;

/// Routes the lock screen buttons to `key` ("play", "pause", "toggle", "next", "prev") and the
/// scrubber to `seek`. Call once, on the main thread.
pub fn commands(key: impl Fn(&'static str) + Clone + 'static, seek: impl Fn(f64) + 'static) {
    unsafe {
        let center = MPRemoteCommandCenter::sharedCommandCenter();
        for (cmd, name) in [
            (center.playCommand(), "play"),
            (center.pauseCommand(), "pause"),
            (center.togglePlayPauseCommand(), "toggle"),
            (center.nextTrackCommand(), "next"),
            (center.previousTrackCommand(), "prev"),
        ] {
            let key = key.clone();
            cmd.setEnabled(true);
            let _ = cmd.addTargetWithHandler(&RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| {
                key(name);
                MPRemoteCommandHandlerStatus::Success
            }));
        }
        let cmd = center.changePlaybackPositionCommand();
        cmd.setEnabled(true);
        let _ = cmd.addTargetWithHandler(&RcBlock::new(move |e: NonNull<MPRemoteCommandEvent>| {
            seek(e.cast::<MPChangePlaybackPositionCommandEvent>().as_ref().positionTime());
            MPRemoteCommandHandlerStatus::Success
        }));
    }
}

/// Shows the song on the lock screen; `None` (stopped) clears it.
pub fn now_playing(title: &str, artist: &str, album: &str, secs: f64, playback: Option<(bool, f64)>) {
    unsafe {
        let center = MPNowPlayingInfoCenter::defaultCenter();
        let Some((playing, pos)) = playback else { return center.setNowPlayingInfo(None) };
        let (t, a, al) = (NSString::from_str(title), NSString::from_str(artist), NSString::from_str(album));
        let (d, p, r) = (NSNumber::new_f64(secs), NSNumber::new_f64(pos), NSNumber::new_f64(if playing { 1.0 } else { 0.0 }));
        let keys = [
            MPMediaItemPropertyTitle,
            MPMediaItemPropertyArtist,
            MPMediaItemPropertyAlbumTitle,
            MPMediaItemPropertyPlaybackDuration,
            MPNowPlayingInfoPropertyElapsedPlaybackTime,
            MPNowPlayingInfoPropertyPlaybackRate,
        ];
        let values: [&AnyObject; 6] = [&t, &a, &al, &d, &p, &r];
        center.setNowPlayingInfo(Some(&NSDictionary::from_slices(&keys, &values)));
    }
}
