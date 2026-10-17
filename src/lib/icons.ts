// Material Symbols Rounded (filled), inlined so they inherit currentColor.
import play from "@material-symbols/svg-400/rounded/play_arrow-fill.svg?raw";
import pause from "@material-symbols/svg-400/rounded/pause-fill.svg?raw";
import next from "@material-symbols/svg-400/rounded/skip_next-fill.svg?raw";
import prev from "@material-symbols/svg-400/rounded/skip_previous-fill.svg?raw";
import shuffle from "@material-symbols/svg-400/rounded/shuffle-fill.svg?raw";
import repeat from "@material-symbols/svg-400/rounded/repeat-fill.svg?raw";
import repeatOne from "@material-symbols/svg-400/rounded/repeat_one-fill.svg?raw";
import volume from "@material-symbols/svg-400/rounded/volume_up-fill.svg?raw";
import search from "@material-symbols/svg-400/rounded/search-fill.svg?raw";
import back from "@material-symbols/svg-400/rounded/arrow_back-fill.svg?raw";
import collapse from "@material-symbols/svg-400/rounded/keyboard_arrow_down-fill.svg?raw";
import note from "@material-symbols/svg-400/rounded/music_note-fill.svg?raw";
import album from "@material-symbols/svg-400/rounded/album-fill.svg?raw";
import artist from "@material-symbols/svg-400/rounded/person-fill.svg?raw";
import playlist from "@material-symbols/svg-400/rounded/queue_music-fill.svg?raw";
import cloud from "@material-symbols/svg-400/rounded/cloud-fill.svg?raw";
import download from "@material-symbols/svg-400/rounded/download-fill.svg?raw";
import downloadDone from "@material-symbols/svg-400/rounded/download_done-fill.svg?raw";
import addFolder from "@material-symbols/svg-400/rounded/create_new_folder-fill.svg?raw";
import add from "@material-symbols/svg-400/rounded/add-fill.svg?raw";
import close from "@material-symbols/svg-400/rounded/close-fill.svg?raw";
import lyrics from "@material-symbols/svg-400/rounded/lyrics-fill.svg?raw";
import playlistAdd from "@material-symbols/svg-400/rounded/playlist_add-fill.svg?raw";

import minimize from "@material-symbols/svg-400/rounded/minimize-fill.svg?raw";
import maximize from "@material-symbols/svg-400/rounded/crop_square.svg?raw";
import restore from "@material-symbols/svg-400/rounded/filter_none.svg?raw";

export const icons = {
  minimize,
  maximize,
  restore,
  play,
  pause,
  next,
  prev,
  shuffle,
  repeat,
  repeatOne,
  volume,
  search,
  back,
  collapse,
  note,
  album,
  artist,
  playlist,
  cloud,
  download,
  downloadDone,
  addFolder,
  add,
  close,
  lyrics,
  playlistAdd,
};

export type IconName = keyof typeof icons;
