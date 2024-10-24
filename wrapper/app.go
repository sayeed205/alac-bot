package wrapper

import (
	eerrors "errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strings"

	mp4tag "github.com/Sorrow446/go-mp4tag"
	"github.com/abema/go-mp4"
	tg "gopkg.in/telebot.v4"
)

var (
	forbiddenNames = regexp.MustCompile(`[\\/<>:"|?*]`)
)

func App(urlStr string, folder string, bot *tg.Bot, ctx tg.Context, msg *tg.Message) (*AutoSong, *os.File, error) {
	authToken, err := GetToken()
	if err != nil {
		fmt.Println("Error getting auth token :", err)
		return nil, nil, err
	}
	//urlStr := "https://music.apple.com/in/album/never-gonna-give-you-up/1559523357?i=1559523359"
	//userToken := os.Getenv("MEDIA_USER_TOKEN")

	meta, err := GetSongMeta(urlStr, authToken)
	if err != nil {
		fmt.Println("Error getting song metadata :", err)
		return nil, nil, err
	}
	//album := meta.Relationships.Albums.Data[0]

	msg, err = bot.Edit(msg, "Found information")
	if err != nil {
		return nil, nil, err
	}

	if meta.Attributes.ExtendedAssetUrls["enhancedHls"] == "" {
		return nil, nil, eerrors.New("ALAC not available")
	}
	enhancedHls, err := GetEnhanceHls(meta.ID)
	if err != nil {
		fmt.Println(err)
	}
	if strings.HasSuffix(enhancedHls, "m3u8") {
		meta.Attributes.ExtendedAssetUrls["enhancedHls"] = enhancedHls
	}

	songName := fmt.Sprintf("%d.%d. %s - %s", meta.Attributes.DiscNumber, meta.Attributes.TrackNumber, meta.Attributes.Name, meta.Attributes.ArtistName) // 1.1. Never Go... - Rik As....
	songName = fmt.Sprintf("%s.m4a", forbiddenNames.ReplaceAllString(songName, "_"))

	lrc := ""
	if os.Getenv("MEDIA_USER_TOKEN") != "" {
		lrc, err = GetLyrics(urlStr, authToken)
		if err != nil {
			fmt.Printf("Failed to parse lyrics: %s \n", err)
		}
	}
	//fmt.Println(lrc)
	//err = os.WriteFile(fmt.Sprintf("%d.lrc", lrc), []byte(lrc), 0644)
	//if err != nil {
	//	fmt.Printf("Failed to write lyrics to file: %s\n", err)
	//	//return
	//}

	// todo)) check if file already exists
	msg, err = bot.Edit(msg, "Extracting "+songName)
	if err != nil {
		return nil, nil, err
	}
	fmt.Println("Extracting ", songName)

	trackUrl, keys, err := ExtractMedia(meta.Attributes.ExtendedAssetUrls["enhancedHls"])
	if err != nil {
		fmt.Println("\u26A0 Failed to extract info from manifest:", err)
		_, _ = bot.Edit(msg, "\u26A0 Failed to extract info from manifest:")
		return nil, nil, err
	}

	info, err := extractSong(trackUrl)
	if err != nil {
		fmt.Println("Failed to extract track.", err)
		_, _ = bot.Edit(msg, "\u26A0 Failed to extract track.")
		return nil, nil, err
	}

	msg, err = bot.Edit(msg, "Extracted "+songName+"...")
	if err != nil {
		return nil, nil, err
	}

	samplesOk := true
	for samplesOk {
		var totalSize int64 = 0
		for _, i := range info.samples {
			totalSize += int64(len(i.data))
			if int(i.descIndex) >= len(keys) {
				fmt.Println("Decryption size mismatch.")
				samplesOk = false
			}
		}
		info.totalDataSize = totalSize
		break
	}
	if !samplesOk {
		return nil, nil, err
	}

	msg, err = bot.Edit(msg, "Decrypting "+meta.Attributes.Name+"...")
	decrypted, err := decryptSong(info, keys, meta, bot, ctx, msg)

	if err != nil {
		fmt.Println("Failed to decrypt song.", err)
		return nil, nil, err
	}

	err = os.MkdirAll(folder, os.ModePerm)
	if err != nil {
		fmt.Println("Failed to create folder:", err)
		return nil, nil, err
	}

	file := filepath.Join(folder, songName)
	create, err := os.Create(file)
	if err != nil {
		fmt.Println("Error creating file :", err)
		return nil, nil, err
	}
	defer create.Close()

	err = WriteM4a(mp4.NewWriter(create), info, meta, decrypted)
	if err != nil {
		fmt.Println("Failed to write m4a.", err)
		return nil, nil, err
	}

	artwork := meta.Attributes.Artwork
	coverUrl := strings.Replace(artwork.URL, "{w}x{h}", fmt.Sprintf("%dx%d", artwork.Width, artwork.Height), -1)

	resp, err := http.Get(coverUrl)
	if err != nil {
		return nil, nil, err
	}
	defer resp.Body.Close()

	cover, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, nil, err
	}
	mp4t, err := mp4tag.Open(file)
	if err != nil {
		return nil, nil, err
	}
	defer mp4t.Close()

	//albumID, err := strconv.ParseInt(album.ID, 10, 32)
	//if err != nil {
	//	return nil, nil, err
	//}
	tags := &mp4tag.MP4Tags{
		Pictures: []*mp4tag.MP4Picture{{Data: cover}},
		Lyrics:   lrc,
		//AlbumArtist:   album.Attributes.ArtistName,
		//Artist:        meta.Attributes.ArtistName,
		//Composer:      meta.Attributes.ComposerName,
		//Copyright:     album.Attributes.Copyright,
		//ItunesAlbumID: int32(albumID),
		//Date:          meta.Attributes.ReleaseDate,
	}

	err = mp4t.Write(tags, []string{})
	if err != nil {
		return nil, nil, err
	}

	return meta, create, err
}
