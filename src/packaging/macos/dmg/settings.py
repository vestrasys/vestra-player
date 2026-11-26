# dmgbuild settings for the Vestra Player DMG (dmgbuild executes this file;
# the version is pinned in requirements.txt next to it).
#
#   python3 -m dmgbuild -s settings.py \
#     -D app=<Vestra Player.app> -D background=<this dir>/background.png \
#     "Vestra Player" <out.dmg>
#
# dmgbuild does not define __file__ here, so the caller passes both paths.
# Retina: dmgbuild finds background@2x.png next to background.png and joins
# the two into one multi-resolution TIFF (tiffutil -cathidpicheck) inside
# the image, so only the two PNG files are kept here; background.svg is
# their source.
#
# The window: 660x400 points, the size of background.png (dmgbuild's
# window_rect size, as electron-builder sets it from the background);
# icons 128 points, centred at (170, 185) and (490, 185) in window
# coordinates, as background.svg draws them.

import os.path

app = defines["app"]  # noqa: F821 (dmgbuild provides `defines`)
background = defines["background"]  # noqa: F821

# As the plain hdiutil path (VESTRA_DMG_PLAIN=1 in bundle-macos.mjs) makes it.
format = "ULMO"
filesystem = "HFS+"

files = [app]
symlinks = {"Applications": "/Applications"}

show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False

window_rect = ((200, 120), (660, 400))
default_view = "icon-view"
show_icon_preview = False
include_icon_view_settings = True
include_list_view_settings = False
arrange_by = None
icon_size = 128
text_size = 13
label_pos = "bottom"

icon_locations = {
    os.path.basename(app.rstrip("/")): (170, 185),
    "Applications": (490, 185),
}
