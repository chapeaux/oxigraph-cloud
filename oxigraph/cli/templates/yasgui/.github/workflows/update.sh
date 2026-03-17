TAG_NAME=$(curl -sL https://api.github.com/repos/zazuko/Yasgui/releases/latest | jq -r ".tag_name")
curl -L "https://github.com/zazuko/Yasgui/releases/download/$TAG_NAME/build.tar.gz" | tar -zx
mv build/yasgui.min.js .
mv build/yasgui.min.js.map .
mv build/yasgui.min.css .
mv build/yasgui.min.js.LICENSE.txt .
rm -rf build
git config --global user.name "github-actions[bot]"
git config --global user.email "username@users.noreply.github.com"
git commit -a -m "Update with $TAG_NAME"
git tag -a "$TAG_NAME" -m "$TAG_NAME"
git push
