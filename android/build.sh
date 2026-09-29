#!/usr/bin/env bash
# Build the APK without Gradle: cargo-ndk → javac → d8 → aapt2 → zipalign → apksigner.
# Needs ANDROID_HOME, ANDROID_NDK_HOME, a JDK, cargo-ndk and zip — all provided by
# `nix develop` locally and by .github/workflows/ci.yml in CI.
# Output: target/apk/rust-kb.apk
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
OUT=$ROOT/target/apk
API=${PLATFORM_API:-35}
MIN_SDK=26
VERSION_CODE=${VERSION_CODE:-1}
VERSION_NAME=${VERSION_NAME:-0.1.0}
BT=$(ls -d "$ANDROID_HOME"/build-tools/* | sort -V | tail -1)
JAR=$ANDROID_HOME/platforms/android-$API/android.jar

# Nothing wholesale is deleted: the models in assets are replaced in place
# (written beside and renamed), so a program reading them keeps going. Only
# the compiled classes are cleared — one of a deleted source would get in.
mkdir -p "$OUT"/{classes,dex,assets,lib}
find "$OUT/classes" -name '*.class' -delete

echo "== native library (arm64-v8a)"
cargo ndk -t arm64-v8a -P "$MIN_SDK" -o "$OUT/lib" build --release -p kbime

echo "== dictionary"
if [ ! -s data/lexicon.tsv ]; then
    echo "data/lexicon.tsv missing: run tools/fetch_sources.sh && tools/build_lexicon.sh" >&2
    exit 1
fi
# Priors in steps of 0.05 nat (the same corrections, 1.3 MB less), and each
# word's grammar id — one structure for words, frequency and grammar
# (kbcore::dict); the context model then keeps what each id means.
GRAMMAR=
if [ -s data/classes.tsv ] && [ -s data/word_readings.tsv ]; then
    GRAMMAR="data/classes.tsv data/word_readings.tsv"
fi
cargo run --release -q -p index-builder -- data/lexicon.tsv "$OUT/assets/dict.fst" --quantum 50 \
    $([ -n "$GRAMMAR" ] && echo --grammar $GRAMMAR)
# The context model is optional: packaged when tools/build_bigrams.py made it.
# Names and abbreviations written with capitals (tools/proper_nouns.py), optional.
if [ -s data/proper.tsv ]; then
    # Obscene words are flagged there too (tools/offensive.py).
    ${PYTHON:-python3} tools/offensive.py base data/lexicon.tsv > "$OUT/offensive.txt"
    cargo run --release -q -p index-builder -- --casing data/proper.tsv "$OUT/assets/casing.fst" "$OUT/offensive.txt"
fi
# Lemma vectors (tools/lemma_vectors.py → data/lemma/vectors.npz), optional:
# the vectors beside the context model, each word's lemma id in it.
if [ -s data/lemma/vectors.npz ] && [ -s data/lemma/words.tsv ]; then
    ${PYTHON:-python3} tools/lemma_export.py data/lemma/vectors.npz \
        --bin "$OUT/assets/lemmas.bin.part" --ids data/lemma/ids.tsv
    mv "$OUT/assets/lemmas.bin.part" "$OUT/assets/lemmas.bin"
else
    rm -f "$OUT/assets/lemmas.bin"
fi
if [ -s data/bigrams.tsv ]; then
    cargo run --release -q -p index-builder -- --bigrams data/bigrams.tsv "$OUT/assets/bigrams.fst" \
        $([ -s data/endings.tsv ] && echo --endings data/endings.tsv) \
        $([ -s data/tag_pairs.tsv ] && echo --classes data/classes.tsv data/class_tags.tsv data/tag_pairs.tsv) \
        $([ -s data/readings.tsv ] && echo --readings data/word_readings.tsv data/readings.tsv) \
        $([ -n "$GRAMMAR" ] && echo --grammar-table $GRAMMAR) \
        $([ -s data/frames.tsv ] && echo --frames data/frames.tsv) \
        $([ -s data/topic_words.tsv ] && echo --topics data/topic_words.tsv --topic-pairs data/topic_pairs.tsv) \
        $([ -s data/commas.tsv ] && echo --commas data/commas.tsv) \
        $([ -s data/yo.tsv ] && echo --yo data/yo.tsv) \
        $([ -s "$OUT/assets/lemmas.bin" ] && echo --lemmas data/lemma/ids.tsv) \
        $([ -s data/rules.tsv ] && echo --rules data/confusions.tsv data/rules.tsv)
fi

# Language packs (tools/lang/<code>.sh): each its own dictionary, context
# model and capitals, installed by the keyboard when the language is on.
for lang in de fr es pt; do
    d=data/$lang
    [ -s "$d/lexicon.tsv" ] || continue
    mkdir -p "$OUT/assets/$lang"
    cargo run --release -q -p index-builder -- "$d/lexicon.tsv" "$OUT/assets/$lang/dict.fst" --quantum 50
    if [ -s "$d/proper.tsv" ]; then
        python3 tools/offensive.py "$lang" "$d/lexicon.tsv" > "$OUT/$lang-offensive.txt"
        cargo run --release -q -p index-builder -- --casing "$d/proper.tsv" "$OUT/assets/$lang/casing.fst" \
            "$OUT/$lang-offensive.txt"
    fi
    if [ -s "$d/bigrams.tsv" ]; then
        cargo run --release -q -p index-builder -- --bigrams "$d/bigrams.tsv" "$OUT/assets/$lang/bigrams.fst" \
            $([ -s "$d/endings.tsv" ] && echo --endings "$d/endings.tsv") \
            $([ -s "$d/rules.tsv" ] && echo --rules "$d/confusions.tsv" "$d/rules.tsv")
    fi
done

# The licenses and attributions of everything packaged (shown in settings).
{
    echo "Rust KB — https://github.com/TimSalguz/rust-android-kb"
    echo "Code: MIT OR Apache-2.0."
    echo
    cat data/README.md
    for f in data/SCOWL-COPYRIGHT data/HUNSPELL-RU-COPYRIGHT; do
        if [ -s "$f" ]; then echo; echo "== $f"; cat "$f"; fi
    done
    echo
    echo "== Emoji list"
    echo "From Unicode's emoji-test.txt (Emoji 17.0), © Unicode, Inc., under the"
    echo "Unicode License v3: https://www.unicode.org/license.txt"
    for lang in de fr es pt; do
        [ -d "$OUT/assets/$lang" ] || continue
        for f in data/$lang/README.md data/$lang/NOTICE data/$lang/*-COPYRIGHT data/$lang/*-NOTICE; do
            if [ -s "$f" ]; then echo; echo "== $f"; cat "$f"; fi
        done
    done
} > "$OUT/assets/NOTICE.txt"

echo "== java"
javac --release 11 -Xlint:-options -classpath "$JAR" -d "$OUT/classes" $(find android/java -name '*.java')
"$BT/d8" --release --min-api "$MIN_SDK" --lib "$JAR" --output "$OUT/dex" $(find "$OUT/classes" -name '*.class')

echo "== resources + manifest"
"$BT/aapt2" compile --dir android/res -o "$OUT/res.zip"
"$BT/aapt2" link -o "$OUT/base.apk" -I "$JAR" --manifest android/AndroidManifest.xml \
    --min-sdk-version "$MIN_SDK" --target-sdk-version "$API" \
    --version-code "$VERSION_CODE" --version-name "$VERSION_NAME" \
    -A "$OUT/assets" -0 fst "$OUT/res.zip"

echo "== package"
cp "$OUT/base.apk" "$OUT/unaligned.apk"
(cd "$OUT/dex" && zip -q -X "$OUT/unaligned.apk" classes.dex)
# Native libs stored uncompressed and 16 KB-aligned so Android can map them in place.
(cd "$OUT" && zip -q -X -0 -r "$OUT/unaligned.apk" lib)
"$BT/zipalign" -f -P 16 4 "$OUT/unaligned.apk" "$OUT/aligned.apk"

# A fixed debug key, so each new build installs over the previous one.
KS=${KEYSTORE:-$ROOT/android/debug.keystore}
if [ ! -f "$KS" ]; then
    keytool -genkeypair -keystore "$KS" -storepass android -keypass android -alias androiddebugkey \
        -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Android Debug,O=Android,C=US"
fi
"$BT/apksigner" sign --ks "$KS" --ks-pass pass:android --key-pass pass:android \
    --out "$OUT/rust-kb.apk" "$OUT/aligned.apk"
ls -la "$OUT/rust-kb.apk"
