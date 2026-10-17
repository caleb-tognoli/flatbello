#!/bin/sh
# Copies the Homebrew dylibs that Flatbello.app's binary needs into Contents/Frameworks,
# rewrites their install names to @rpath, signs the bundle ad hoc and builds a .dmg.
#   tools/package/macos-bundle-occt.sh path/to/Flatbello.app out.dmg
set -eu
app=$1
dmg=$2
bin="$app/Contents/MacOS/flatbello"
fw="$app/Contents/Frameworks"
mkdir -p "$fw"

# Non-system dependencies of a Mach-O file, one path per line.
deps() {
    otool -L "$1" | tail -n +2 | awk '{print $1}' | grep -E '^/(opt|usr/local)/' || true
}

queue="$bin"
done_list=""
while [ -n "$queue" ]; do
    file=${queue%%:*}
    [ "$queue" = "$file" ] && queue="" || queue=${queue#*:}
    for dep in $(deps "$file"); do
        name=$(basename "$dep")
        if [ ! -f "$fw/$name" ]; then
            cp -L "$dep" "$fw/$name"
            chmod u+w "$fw/$name"
            install_name_tool -id "@rpath/$name" "$fw/$name"
            queue="${queue:+$queue:}$fw/$name"
        fi
        install_name_tool -change "$dep" "@rpath/$name" "$file"
    done
    done_list="$done_list $file"
done
install_name_tool -add_rpath "@executable_path/../Frameworks" "$bin" 2>/dev/null || true
codesign --force --deep --sign - "$app"
hdiutil create -volname Flatbello -srcfolder "$app" -ov -format UDZO "$dmg"
echo "bundled:$done_list"
