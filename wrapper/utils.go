package wrapper

import (
	"bufio"
	"bytes"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/abema/go-mp4"
	tg "gopkg.in/telebot.v4"
	"io"
	"log"
	"math"
	"net"
	"net/http"
	"net/url"
	"os"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/beevik/etree"
	"github.com/grafov/m3u8"
)

const (
	defaultId   = "0"
	prefetchKey = "skd://itunes.apple.com/P000000000/s1/e1"
)

//func Extract

func ExtractUrlMeta(inputURL string) (*URLMeta, error) {
	// Define regex patterns to match album, song, and playlist URLs
	// Pattern for album or song: https://music.apple.com/<storefront>/album|song/<name>/<id>
	// Pattern for playlist: https://music.apple.com/library/playlist/<id>
	reAlbumOrSong := regexp.MustCompile(`https://music\.apple\.com/(?P<storefront>[a-z]{2})/(?P<type>album|song)/.*/(?P<id>[0-9]+)`)
	rePlaylist := regexp.MustCompile(`https://music\.apple\.com/library/playlist/(?P<id>[a-zA-Z0-9\.]+)`)

	// First, try matching the playlist URL
	if matches := rePlaylist.FindStringSubmatch(inputURL); matches != nil {
		// Extract the playlist ID and return the metadata
		id := matches[1]
		return &URLMeta{
			Storefront: "", // Playlists don't have a storefront
			URLType:    "playlists",
			ID:         id,
		}, nil
	}

	// Next, try matching the album or song URL
	if matches := reAlbumOrSong.FindStringSubmatch(inputURL); matches != nil {
		// Extract the storefront, type (album or song), and id
		storefront := matches[1]
		urlType := matches[2]
		id := matches[3]

		// Check if it's an album URL with the "i" query parameter (song within album)
		if urlType == "album" {
			u, err := url.Parse(inputURL)
			if err != nil {
				return nil, fmt.Errorf("invalid URL: %v", err)
			}

			// If the query contains "i", use the "i" value as the song ID
			if songID := u.Query().Get("i"); songID != "" {
				id = songID
				urlType = "song" // Treat it as a song since "i" parameter is found
			}
		}

		// Return the parsed metadata
		return &URLMeta{
			Storefront: storefront,
			URLType:    urlType + "s",
			ID:         id,
		}, nil
	}

	return nil, fmt.Errorf("invalid Apple Music URL format")
}

func GetSongMeta(urlStr string, token string) (*AutoSong, error) {
	urlMeta, err := ExtractUrlMeta(urlStr)

	if err != nil {
		return nil, err
	}

	URL := fmt.Sprintf("https://amp-api.music.apple.com/v1/catalog/%s/%s/%s", urlMeta.Storefront, urlMeta.URLType, urlMeta.ID)

	fmt.Println("Hitting :", URL)

	req, err := http.NewRequest("GET", URL, nil)
	if err != nil {
		return nil, err
	}

	// Set headers for the request
	req.Header.Set("Authorization", fmt.Sprintf("Bearer %s", token))
	req.Header.Set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
	req.Header.Set("Origin", "https://music.apple.com")

	// Prepare query parameters
	query := url.Values{}
	//query.Set("omit[resource]", "autos")
	query.Set("include", "albums,explicit")
	//query.Set("include[songs]", "artists")
	//query.Set("fields[artists]", "name,artwork")
	query.Set("extend", "extendedAssetUrls")
	//query.Set("fields[albums]", "artistName,artwork,name,releaseDate,url,relationships")
	//query.Set("fields[record-labels]", "name")
	query.Set("l", os.Getenv("LANGUAGE"))
	req.URL.RawQuery = query.Encode()

	// Make the HTTP request
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	// Check for a successful response
	if resp.StatusCode != http.StatusOK {
		return nil, errors.New(resp.Status)
	}

	// Decode the response body into the SongResponse struct
	var songResponse SongResponse
	if err := json.NewDecoder(resp.Body).Decode(&songResponse); err != nil {
		return nil, err
	}

	for _, d := range songResponse.Data {
		if d.ID == urlMeta.ID {
			return &d, nil
		}
	}

	return nil, nil
}

func GetEnhanceHls(songId string) (string, error) {
	var EnhancedHls string
	conn, err := net.Dial("tcp", os.Getenv("DEVICE_URL"))

	if err != nil {
		log.Println("Error connecting to device:", err)
		return "none", err
	}

	defer conn.Close()

	adamIDBuffer := []byte(songId)
	lengthBuffer := []byte{byte(len(adamIDBuffer))}

	// Write length and adamID to the connection
	_, err = conn.Write(lengthBuffer)
	if err != nil {
		fmt.Println("Error writing length to device:", err)
		return "none", err
	}

	_, err = conn.Write(adamIDBuffer)
	if err != nil {
		fmt.Println("Error writing adamID to device:", err)
		return "none", err
	}

	// Read the response (URL) from the device
	response, err := bufio.NewReader(conn).ReadBytes('\n')
	if err != nil {
		fmt.Println("Error reading response from device:", err)
		return "none", err
	}

	// Trim any newline characters from the response

	response = bytes.TrimSpace(response)
	if len(response) > 0 {
		EnhancedHls = string(response)
	} else {
		fmt.Println("Received an empty response")
	}

	return EnhancedHls, nil
}

func GetLyrics(urlStr string, token string) (string, error) {
	userToken := os.Getenv("MEDIA_USER_TOKEN")
	meta, err := ExtractUrlMeta(urlStr)
	if err != nil {
		return "", err
	}

	req, err := http.NewRequest("GET",
		fmt.Sprintf("https://amp-api.music.apple.com/v1/catalog/%s/songs/%s/syllable-lyrics", meta.Storefront, meta.ID), nil)

	req.Header.Set("Origin", "https://music.apple.com")
	req.Header.Set("Referer", "https://music.apple.com/")
	req.Header.Set("Authorization", fmt.Sprintf("Bearer %s", token))
	cookie := http.Cookie{Name: "media-user-token", Value: userToken}
	req.AddCookie(&cookie)
	do, err := http.DefaultClient.Do(req)
	if err != nil {
		return "", err
	}
	defer do.Body.Close()

	obj := new(SongLyrics)
	err = json.NewDecoder(do.Body).Decode(&obj)
	ttml := ""
	if obj.Data != nil {
		ttml = obj.Data[0].Attributes.Ttml
	} else {
		return "", errors.New("failed to get lyrics")
	}

	parsedTTML := etree.NewDocument()
	err = parsedTTML.ReadFromString(ttml)
	if err != nil {
		return "", err
	}

	var lrcLines []string
	timingAttr := parsedTTML.FindElement("tt").SelectAttr("itunes:timing")
	if timingAttr != nil {
		if timingAttr.Value == "Word" {
			//lrc, err := conventSyllableTTMLToLRC(ttml)
			for _, div := range parsedTTML.FindElement("tt").FindElement("body").FindElements("div") {
				lineStart := parseTime(div.SelectAttr("begin").Value)
				lineEnd := parseTime(div.SelectAttr("end").Value)
				for _, line := range div.ChildElements() {
					v := line.SelectAttr("ttm:agent").Value
					var lrcSyllables []string
					var i int = 0

					for _, syllable := range line.Child {
						if _, ok := syllable.(*etree.CharData); ok {
							if i > 0 {
								lrcSyllables = append(lrcSyllables, " ")
								continue
							}
							continue
						}
						lyric := syllable.(*etree.Element)
						if lyric.SelectAttr("begin") == nil {
							continue
						}
						beginTime := parseTime(lyric.SelectAttr("begin").Value)

						var text string
						if lyric.SelectAttr("text") == nil {
							var textTmp []string
							for _, span := range lyric.Child {
								if _, ok := span.(*etree.CharData); ok {
									textTmp = append(textTmp, span.(*etree.CharData).Data)
								} else {
									textTmp = append(textTmp, span.(*etree.Element).Text())
								}
							}
							text = strings.Join(textTmp, "")
						} else {
							text = lyric.SelectAttr("text").Value
						}
						lrcSyllables = append(lrcSyllables, fmt.Sprintf("<%s>%s", beginTime, text))
						i += 1
					}

					lrcLines = append(lrcLines, "["+lineStart+"]"+v+": "+strings.Join(lrcSyllables, "")+"<"+lineEnd+">")
				}
			}
			return strings.Join(lrcLines, "\n"), nil
		}
		if timingAttr.Value == "None" {
			for _, p := range parsedTTML.FindElements("//p") {
				line := p.Text()
				line = strings.TrimSpace(line)
				if line != "" {
					lrcLines = append(lrcLines, line)
				}
			}
			return strings.Join(lrcLines, "\n"), nil
		}
	}

	for _, item := range parsedTTML.FindElement("tt").FindElement("body").ChildElements() {
		for _, lyric := range item.ChildElements() {
			var h, m, s, ms int
			if lyric.SelectAttr("begin") == nil {
				return "", errors.New("no synchronised lyrics")
			}
			if strings.Contains(lyric.SelectAttr("begin").Value, ":") {
				_, err = fmt.Sscanf(lyric.SelectAttr("begin").Value, "%d:%d:%d.%d", &h, &m, &s, &ms)
				if err != nil {
					_, err = fmt.Sscanf(lyric.SelectAttr("begin").Value, "%d:%d.%d", &m, &s, &ms)
					if err != nil {
						_, err = fmt.Sscanf(lyric.SelectAttr("begin").Value, "%d:%d", &m, &s)
					}
					h = 0
				}
			} else {
				_, err = fmt.Sscanf(lyric.SelectAttr("begin").Value, "%d.%d", &s, &ms)
				h, m = 0, 0
			}
			if err != nil {
				return "", err
			}
			var text string
			if lyric.SelectAttr("text") == nil {
				var textTmp []string
				for _, span := range lyric.Child {
					if _, ok := span.(*etree.CharData); ok {
						textTmp = append(textTmp, span.(*etree.CharData).Data)
					} else {
						textTmp = append(textTmp, span.(*etree.Element).Text())
					}
				}
				text = strings.Join(textTmp, "")
			} else {
				text = lyric.SelectAttr("text").Value
			}
			m += h * 60
			ms = ms / 10
			lrcLines = append(lrcLines, fmt.Sprintf("[%02d:%02d.%02d]%s", m, s, ms, text))
		}
	}
	return strings.Join(lrcLines, "\n"), nil
}

func parseTime(time string) string {
	parts := strings.Split(time, ":")
	if len(parts) == 1 {
		// If no colon, assume it's in the format of seconds.milliseconds
		seconds, err := strconv.ParseFloat(parts[0], 64)
		if err != nil {
			return time // Return unchanged if parsing fails
		}
		if seconds < 60 {
			return fmt.Sprintf("00:%06.3f", seconds) // Pad to ensure 2 digits for seconds
		}
		return time // If seconds >= 60, no need to modify
	} else if len(parts) == 2 {
		minutes := parts[0]
		seconds, err := strconv.ParseFloat(parts[1], 64)
		if err != nil {
			return time // Return unchanged if parsing fails
		}
		return fmt.Sprintf("%s:%06.3f", minutes, seconds) // Format as MM:SS.SSS
	}

	return time // Return unchanged if no match
}

func ExtractMedia(urlStr string) (string, []string, error) {
	masterUrl, err := url.Parse(urlStr)
	if err != nil {
		return "", nil, err
	}

	resp, err := http.Get(urlStr)
	if err != nil {
		return "", nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", nil, errors.New(resp.Status)
	}

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", nil, err
	}
	masterString := string(body)
	from, listType, err := m3u8.DecodeFrom(strings.NewReader(masterString), true)
	if err != nil || listType != m3u8.MASTER {
		return "", nil, errors.New("m3u8 not of master type")
	}
	master := from.(*m3u8.MasterPlaylist)
	var streamUrl *url.URL
	sort.Slice(master.Variants, func(i, j int) bool {
		return master.Variants[i].AverageBandwidth > master.Variants[j].AverageBandwidth
	})

	for _, variant := range master.Variants {
		if variant.Codecs == "alac" {
			split := strings.Split(variant.Audio, "-")
			length := len(split)
			lengthInt, err := strconv.Atoi(split[length-2])
			if err != nil {
				return "", nil, err
			}
			if lengthInt <= 192000 {
				fmt.Printf("%s-bit / %s Hz\n", split[length-1], split[length-2])
				streamUrlTemp, err := masterUrl.Parse(variant.URI)
				if err != nil {
					panic(err)
				}
				streamUrl = streamUrlTemp
				break
			}
		}
	}

	if streamUrl == nil {
		return "", nil, errors.New("no codec found")
	}
	var keys []string
	keys = append(keys, prefetchKey)
	streamUrl.Path = strings.TrimSuffix(streamUrl.Path, ".m3u8") + "_m.mp4"
	regex := regexp.MustCompile(`"(skd?://[^"]*)"`)
	matches := regex.FindAllStringSubmatch(masterString, -1)
	for _, match := range matches {
		if strings.HasSuffix(match[1], "c23") || strings.HasSuffix(match[1], "c6") {
			keys = append(keys, match[1])
		}
	}
	return streamUrl.String(), keys, nil
}

func extractSong(url string) (*SongInfo, error) {
	track, err := http.Get(url)
	if err != nil {
		return nil, err
	}
	defer track.Body.Close()
	if track.StatusCode != http.StatusOK {
		return nil, errors.New(track.Status)
	}

	rawSong, err := io.ReadAll(track.Body)
	if err != nil {
		return nil, err
	}
	f := bytes.NewReader(rawSong)

	trex, err := mp4.ExtractBoxWithPayload(f, nil, []mp4.BoxType{
		mp4.BoxTypeMoov(),
		mp4.BoxTypeMvex(),
		mp4.BoxTypeTrex(),
	})
	if err != nil || len(trex) != 1 {
		return nil, err
	}
	trexPay := trex[0].Payload.(*mp4.Trex)

	stbl, err := mp4.ExtractBox(f, nil, []mp4.BoxType{
		mp4.BoxTypeMoov(),
		mp4.BoxTypeTrak(),
		mp4.BoxTypeMdia(),
		mp4.BoxTypeMinf(),
		mp4.BoxTypeStbl(),
	})
	if err != nil || len(stbl) != 1 {
		return nil, err
	}

	var extracted *SongInfo
	enca, err := mp4.ExtractBoxWithPayload(f, stbl[0], []mp4.BoxType{
		mp4.BoxTypeStsd(),
		mp4.BoxTypeEnca(),
	})
	if err != nil {
		return nil, err
	}

	aalac, err := mp4.ExtractBoxWithPayload(f, &enca[0].Info,
		[]mp4.BoxType{BoxTypeAlac()})
	if err != nil || len(aalac) != 1 {
		return nil, err
	}
	extracted = &SongInfo{
		r:         f,
		alacParam: aalac[0].Payload.(*Alac),
	}

	moofs, err := mp4.ExtractBox(f, nil, []mp4.BoxType{
		mp4.BoxTypeMoof(),
	})
	if err != nil || len(moofs) <= 0 {
		return nil, err
	}

	mdats, err := mp4.ExtractBoxWithPayload(f, nil, []mp4.BoxType{
		mp4.BoxTypeMdat(),
	})
	if err != nil || len(mdats) != len(moofs) {
		return nil, err
	}

	for i, moof := range moofs {
		tfhd, err := mp4.ExtractBoxWithPayload(f, moof, []mp4.BoxType{
			mp4.BoxTypeTraf(),
			mp4.BoxTypeTfhd(),
		})
		if err != nil || len(tfhd) != 1 {
			return nil, err
		}
		tfhdPay := tfhd[0].Payload.(*mp4.Tfhd)
		index := tfhdPay.SampleDescriptionIndex
		if index != 0 {
			index--
		}

		truns, err := mp4.ExtractBoxWithPayload(f, moof, []mp4.BoxType{
			mp4.BoxTypeTraf(),
			mp4.BoxTypeTrun(),
		})
		if err != nil || len(truns) <= 0 {
			return nil, err
		}

		mdat := mdats[i].Payload.(*mp4.Mdat).Data
		for _, t := range truns {
			for _, en := range t.Payload.(*mp4.Trun).Entries {
				info := SampleInfo{descIndex: index}

				switch {
				case t.Payload.CheckFlag(0x200):
					info.data = mdat[:en.SampleSize]
					mdat = mdat[en.SampleSize:]
				case tfhdPay.CheckFlag(0x10):
					info.data = mdat[:tfhdPay.DefaultSampleSize]
					mdat = mdat[tfhdPay.DefaultSampleSize:]
				default:
					info.data = mdat[:trexPay.DefaultSampleSize]
					mdat = mdat[trexPay.DefaultSampleSize:]
				}

				switch {
				case t.Payload.CheckFlag(0x100):
					info.duration = en.SampleDuration
				case tfhdPay.CheckFlag(0x8):
					info.duration = tfhdPay.DefaultSampleDuration
				default:
					info.duration = trexPay.DefaultSampleDuration
				}

				extracted.samples = append(extracted.samples, info)
			}
		}
		if len(mdat) != 0 {
			return nil, errors.New("offset mismatch")
		}
	}

	return extracted, nil
}

func (*Alac) GetType() mp4.BoxType {
	return BoxTypeAlac()
}

func (s *SongInfo) Duration() (ret uint64) {
	for i := range s.samples {
		ret += uint64(s.samples[i].duration)
	}
	return
}

func BoxTypeAlac() mp4.BoxType { return mp4.StrToBoxType("alac") }

func init() {
	mp4.AddBoxDef((*Alac)(nil))
}

func decryptSong(info *SongInfo, keys []string, manifest *AutoSong, bot *tg.Bot, ctx tg.Context, msg *tg.Message) ([]byte, error) {
	conn, err := net.Dial("tcp", os.Getenv("DECRYPTION_URL"))
	if err != nil {
		return nil, err
	}
	defer conn.Close()

	var decrypted []byte
	var lastIndex uint32 = math.MaxUint8

	var pd, totalProcessed int
	totalSize := info.totalDataSize

	// Set up ticker for 5 seconds interval updates
	ticker := time.NewTicker(5 * time.Second)
	defer ticker.Stop()

	// Start a goroutine to update the Telegram message every 5 seconds
	go func() {
		for range ticker.C {
			percentage := float64(totalProcessed) / float64(totalSize) * 100
			_, err := bot.Edit(msg, fmt.Sprintf("Decrypting - (%.2f%%)", percentage))
			if err != nil && !strings.Contains(err.Error(), "not modified") {
				fmt.Println("Error updating message:", err)
			}
		}
	}()

	for _, sp := range info.samples {
		if lastIndex != sp.descIndex {
			if len(decrypted) != 0 {
				_, err := conn.Write([]byte{0, 0, 0, 0})
				if err != nil {
					return nil, err
				}
			}
			keyUri := keys[sp.descIndex]
			id := manifest.ID
			if keyUri == prefetchKey {
				id = defaultId
			}

			_, err := conn.Write([]byte{byte(len(id))})
			if err != nil {
				return nil, err
			}
			_, err = io.WriteString(conn, id)
			if err != nil {
				return nil, err
			}

			_, err = conn.Write([]byte{byte(len(keyUri))})
			if err != nil {
				return nil, err
			}
			_, err = io.WriteString(conn, keyUri)
			if err != nil {
				return nil, err
			}
		}
		lastIndex = sp.descIndex

		err := binary.Write(conn, binary.LittleEndian, uint32(len(sp.data)))
		if err != nil {
			return nil, err
		}

		_, err = conn.Write(sp.data)
		if err != nil {
			return nil, err
		}

		de := make([]byte, len(sp.data))
		_, err = io.ReadFull(conn, de)
		if err != nil {
			return nil, err
		}

		decrypted = append(decrypted, de...)
		pd = len(sp.data)
		totalProcessed += pd
	}

	msg, err = bot.Edit(msg, "Decryption complete")
	if err != nil {
		return nil, err
	}

	_, _ = conn.Write([]byte{0, 0, 0, 0, 0})

	return decrypted, nil
}

func WriteM4a(w *mp4.Writer, info *SongInfo, meta *AutoSong, data []byte) error {
	albums := meta.Relationships.Albums.Data
	artists := meta.Relationships.Artists.Data
	{ // ftyp
		box, err := w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeFtyp()})
		if err != nil {
			return err
		}
		_, err = mp4.Marshal(w, &mp4.Ftyp{
			MajorBrand:   [4]byte{'M', '4', 'A', ' '},
			MinorVersion: 0,
			CompatibleBrands: []mp4.CompatibleBrandElem{
				{CompatibleBrand: [4]byte{'M', '4', 'A', ' '}},
				{CompatibleBrand: [4]byte{'m', 'p', '4', '2'}},
				{CompatibleBrand: mp4.BrandISOM()},
				{CompatibleBrand: [4]byte{0, 0, 0, 0}},
			},
		}, box.Context)
		if err != nil {
			return err
		}
		_, err = w.EndBox()
		if err != nil {
			return err
		}
	}

	const chunkSize uint32 = 5
	duration := info.Duration()
	numSamples := uint32(len(info.samples))
	var stco *mp4.BoxInfo

	{ // moov
		_, err := w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeMoov()})
		if err != nil {
			return err
		}
		box, err := mp4.ExtractBox(info.r, nil, mp4.BoxPath{mp4.BoxTypeMoov()})
		if err != nil {
			return err
		}
		moovOri := box[0]

		{ // mvhd
			_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeMvhd()})
			if err != nil {
				return err
			}

			oriBox, err := mp4.ExtractBoxWithPayload(info.r, moovOri, mp4.BoxPath{mp4.BoxTypeMvhd()})
			if err != nil {
				return err
			}
			mvhd := oriBox[0].Payload.(*mp4.Mvhd)
			if mvhd.Version == 0 {
				mvhd.DurationV0 = uint32(duration)
			} else {
				mvhd.DurationV1 = duration
			}

			_, err = mp4.Marshal(w, mvhd, oriBox[0].Info.Context)
			if err != nil {
				return err
			}

			_, err = w.EndBox()
			if err != nil {
				return err
			}
		}

		{ // trak
			_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeTrak()})
			if err != nil {
				return err
			}

			box, err := mp4.ExtractBox(info.r, moovOri, mp4.BoxPath{mp4.BoxTypeTrak()})
			if err != nil {
				return err
			}
			trakOri := box[0]

			{ // tkhd
				_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeTkhd()})
				if err != nil {
					return err
				}

				oriBox, err := mp4.ExtractBoxWithPayload(info.r, trakOri, mp4.BoxPath{mp4.BoxTypeTkhd()})
				if err != nil {
					return err
				}
				tkhd := oriBox[0].Payload.(*mp4.Tkhd)
				if tkhd.Version == 0 {
					tkhd.DurationV0 = uint32(duration)
				} else {
					tkhd.DurationV1 = duration
				}
				tkhd.SetFlags(0x7)

				_, err = mp4.Marshal(w, tkhd, oriBox[0].Info.Context)
				if err != nil {
					return err
				}

				_, err = w.EndBox()
				if err != nil {
					return err
				}
			}

			{ // mdia
				_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeMdia()})
				if err != nil {
					return err
				}

				box, err := mp4.ExtractBox(info.r, trakOri, mp4.BoxPath{mp4.BoxTypeMdia()})
				if err != nil {
					return err
				}
				mdiaOri := box[0]

				{ // mdhd
					_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeMdhd()})
					if err != nil {
						return err
					}

					oriBox, err := mp4.ExtractBoxWithPayload(info.r, mdiaOri, mp4.BoxPath{mp4.BoxTypeMdhd()})
					if err != nil {
						return err
					}
					mdhd := oriBox[0].Payload.(*mp4.Mdhd)
					if mdhd.Version == 0 {
						mdhd.DurationV0 = uint32(duration)
					} else {
						mdhd.DurationV1 = duration
					}

					_, err = mp4.Marshal(w, mdhd, oriBox[0].Info.Context)
					if err != nil {
						return err
					}

					_, err = w.EndBox()
					if err != nil {
						return err
					}
				}

				{ // hdlr
					oriBox, err := mp4.ExtractBox(info.r, mdiaOri, mp4.BoxPath{mp4.BoxTypeHdlr()})
					if err != nil {
						return err
					}

					err = w.CopyBox(info.r, oriBox[0])
					if err != nil {
						return err
					}
				}

				{ // minf
					_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeMinf()})
					if err != nil {
						return err
					}

					box, err := mp4.ExtractBox(info.r, mdiaOri, mp4.BoxPath{mp4.BoxTypeMinf()})
					if err != nil {
						return err
					}
					minfOri := box[0]

					{ // smhd, dinf
						boxes, err := mp4.ExtractBoxes(info.r, minfOri, []mp4.BoxPath{
							{mp4.BoxTypeSmhd()},
							{mp4.BoxTypeDinf()},
						})
						if err != nil {
							return err
						}

						for _, b := range boxes {
							err = w.CopyBox(info.r, b)
							if err != nil {
								return err
							}
						}
					}

					{ // stbl
						_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeStbl()})
						if err != nil {
							return err
						}

						{ // stsd
							box, err := w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeStsd()})
							if err != nil {
								return err
							}
							_, err = mp4.Marshal(w, &mp4.Stsd{EntryCount: 1}, box.Context)
							if err != nil {
								return err
							}

							{ // alac
								_, err = w.StartBox(&mp4.BoxInfo{Type: BoxTypeAlac()})
								if err != nil {
									return err
								}

								_, err = w.Write([]byte{
									0, 0, 0, 0, 0, 0, 0, 1,
									0, 0, 0, 0, 0, 0, 0, 0})
								if err != nil {
									return err
								}

								err = binary.Write(w, binary.BigEndian, uint16(info.alacParam.NumChannels))
								if err != nil {
									return err
								}

								err = binary.Write(w, binary.BigEndian, uint16(info.alacParam.BitDepth))
								if err != nil {
									return err
								}

								_, err = w.Write([]byte{0, 0})
								if err != nil {
									return err
								}

								err = binary.Write(w, binary.BigEndian, info.alacParam.SampleRate)
								if err != nil {
									return err
								}

								_, err = w.Write([]byte{0, 0})
								if err != nil {
									return err
								}

								box, err := w.StartBox(&mp4.BoxInfo{Type: BoxTypeAlac()})
								if err != nil {
									return err
								}

								_, err = mp4.Marshal(w, info.alacParam, box.Context)
								if err != nil {
									return err
								}

								_, err = w.EndBox()
								if err != nil {
									return err
								}

								_, err = w.EndBox()
								if err != nil {
									return err
								}
							}

							_, err = w.EndBox()
							if err != nil {
								return err
							}
						}

						{ // stts
							box, err := w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeStts()})
							if err != nil {
								return err
							}

							var stts mp4.Stts
							for _, sample := range info.samples {
								if len(stts.Entries) != 0 {
									last := &stts.Entries[len(stts.Entries)-1]
									if last.SampleDelta == sample.duration {
										last.SampleCount++
										continue
									}
								}
								stts.Entries = append(stts.Entries, mp4.SttsEntry{
									SampleCount: 1,
									SampleDelta: sample.duration,
								})
							}
							stts.EntryCount = uint32(len(stts.Entries))

							_, err = mp4.Marshal(w, &stts, box.Context)
							if err != nil {
								return err
							}

							_, err = w.EndBox()
							if err != nil {
								return err
							}
						}

						{ // stsc
							box, err := w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeStsc()})
							if err != nil {
								return err
							}

							if numSamples%chunkSize == 0 {
								_, err = mp4.Marshal(w, &mp4.Stsc{
									EntryCount: 1,
									Entries: []mp4.StscEntry{
										{
											FirstChunk:             1,
											SamplesPerChunk:        chunkSize,
											SampleDescriptionIndex: 1,
										},
									},
								}, box.Context)
							} else {
								_, err = mp4.Marshal(w, &mp4.Stsc{
									EntryCount: 2,
									Entries: []mp4.StscEntry{
										{
											FirstChunk:             1,
											SamplesPerChunk:        chunkSize,
											SampleDescriptionIndex: 1,
										}, {
											FirstChunk:             numSamples/chunkSize + 1,
											SamplesPerChunk:        numSamples % chunkSize,
											SampleDescriptionIndex: 1,
										},
									},
								}, box.Context)
							}

							_, err = w.EndBox()
							if err != nil {
								return err
							}
						}

						{ // stsz
							box, err := w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeStsz()})
							if err != nil {
								return err
							}

							stsz := mp4.Stsz{SampleCount: numSamples}
							for _, sample := range info.samples {
								stsz.EntrySize = append(stsz.EntrySize, uint32(len(sample.data)))
							}

							_, err = mp4.Marshal(w, &stsz, box.Context)
							if err != nil {
								return err
							}

							_, err = w.EndBox()
							if err != nil {
								return err
							}
						}

						{ // stco
							box, err := w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeStco()})
							if err != nil {
								return err
							}

							l := (numSamples + chunkSize - 1) / chunkSize
							_, err = mp4.Marshal(w, &mp4.Stco{
								EntryCount:  l,
								ChunkOffset: make([]uint32, l),
							}, box.Context)

							stco, err = w.EndBox()
							if err != nil {
								return err
							}
						}

						_, err = w.EndBox()
						if err != nil {
							return err
						}
					}

					_, err = w.EndBox()
					if err != nil {
						return err
					}
				}

				_, err = w.EndBox()
				if err != nil {
					return err
				}
			}

			_, err = w.EndBox()
			if err != nil {
				return err
			}
		}

		{ // udta
			ctx := mp4.Context{UnderUdta: true}
			_, err := w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeUdta(), Context: ctx})
			if err != nil {
				return err
			}

			{ // meta
				ctx.UnderIlstMeta = true

				_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeMeta(), Context: ctx})
				if err != nil {
					return err
				}

				_, err = mp4.Marshal(w, &mp4.Meta{}, ctx)
				if err != nil {
					return err
				}

				{ // hdlr
					_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeHdlr(), Context: ctx})
					if err != nil {
						return err
					}

					_, err = mp4.Marshal(w, &mp4.Hdlr{
						HandlerType: [4]byte{'m', 'd', 'i', 'r'},
						Reserved:    [3]uint32{0x6170706c, 0, 0},
					}, ctx)
					if err != nil {
						return err
					}

					_, err = w.EndBox()
					if err != nil {
						return err
					}
				}

				{ // ilst
					ctx.UnderIlst = true

					_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeIlst(), Context: ctx})
					if err != nil {
						return err
					}

					marshalData := func(val interface{}) error {
						_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeData()})
						if err != nil {
							return err
						}

						var boxData mp4.Data
						switch v := val.(type) {
						case string:
							boxData.DataType = mp4.DataTypeStringUTF8
							boxData.Data = []byte(v)
						case uint8:
							boxData.DataType = mp4.DataTypeSignedIntBigEndian
							boxData.Data = []byte{v}
						case uint32:
							boxData.DataType = mp4.DataTypeSignedIntBigEndian
							boxData.Data = make([]byte, 4)
							binary.BigEndian.PutUint32(boxData.Data, v)
						case []byte:
							boxData.DataType = mp4.DataTypeBinary
							boxData.Data = v
						default:
							panic("unsupported value")
						}

						_, err = mp4.Marshal(w, &boxData, ctx)
						if err != nil {
							return err
						}

						_, err = w.EndBox()
						return err
					}

					addMeta := func(tag mp4.BoxType, val interface{}) error {
						_, err = w.StartBox(&mp4.BoxInfo{Type: tag})
						if err != nil {
							return err
						}

						err = marshalData(val)
						if err != nil {
							return err
						}

						_, err = w.EndBox()
						return err
					}

					addExtendedMeta := func(name string, val interface{}) error {
						ctx.UnderIlstFreeMeta = true

						_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxType{'-', '-', '-', '-'}, Context: ctx})
						if err != nil {
							return err
						}

						{
							_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxType{'m', 'e', 'a', 'n'}, Context: ctx})
							if err != nil {
								return err
							}

							_, err = w.Write([]byte{0, 0, 0, 0})
							if err != nil {
								return err
							}

							_, err = io.WriteString(w, "com.apple.iTunes")
							if err != nil {
								return err
							}

							_, err = w.EndBox()
							if err != nil {
								return err
							}
						}

						{
							_, err = w.StartBox(&mp4.BoxInfo{Type: mp4.BoxType{'n', 'a', 'm', 'e'}, Context: ctx})
							if err != nil {
								return err
							}

							_, err = w.Write([]byte{0, 0, 0, 0})
							if err != nil {
								return err
							}

							_, err = io.WriteString(w, name)
							if err != nil {
								return err
							}

							_, err = w.EndBox()
							if err != nil {
								return err
							}
						}

						err = marshalData(val)
						if err != nil {
							return err
						}

						ctx.UnderIlstFreeMeta = false

						_, err = w.EndBox()
						return err
					}

					err = addMeta(mp4.BoxType{'\251', 'n', 'a', 'm'}, meta.Attributes.Name)
					if err != nil {
						return err
					}

					err = addMeta(mp4.BoxType{'s', 'o', 'n', 'm'}, meta.Attributes.Name)
					if err != nil {
						return err
					}
					AlbumName := meta.Attributes.AlbumName
					//if strings.Contains(meta.ID, "pl.") {
					//	if !config.UseSongInfoForPlaylist {
					//		AlbumName = meta.Data[0].Attributes.Name
					//	}
					//}
					err = addMeta(mp4.BoxType{'\251', 'a', 'l', 'b'}, AlbumName)
					if err != nil {
						return err
					}

					err = addMeta(mp4.BoxType{'s', 'o', 'a', 'l'}, AlbumName)
					if err != nil {
						return err
					}

					err = addMeta(mp4.BoxType{'\251', 'A', 'R', 'T'}, meta.Attributes.ArtistName)
					if err != nil {
						return err
					}

					err = addMeta(mp4.BoxType{'s', 'o', 'a', 'r'}, meta.Attributes.ArtistName)
					if err != nil {
						return err
					}

					err = addMeta(mp4.BoxType{'\251', 'p', 'r', 'f'}, meta.Attributes.ArtistName)
					if err != nil {
						return err
					}

					err = addExtendedMeta("PERFORMER", meta.Attributes.ArtistName)
					if err != nil {
						return err
					}

					err = addExtendedMeta("ITUNESALBUMID", albums[0].ID)
					if err != nil {
						return err
					}

					err = addMeta(mp4.BoxType{'\251', 'w', 'r', 't'}, meta.Attributes.ComposerName)
					if err != nil {
						return err
					}

					err = addMeta(mp4.BoxType{'s', 'o', 'c', 'o'}, meta.Attributes.ComposerName)
					if err != nil {
						return err
					}

					err = addMeta(mp4.BoxType{'\251', 'd', 'a', 'y'}, meta.Attributes.ReleaseDate)
					if err != nil {
						return err
					}

					err = addExtendedMeta("RELEASETIME", meta.Attributes.ReleaseDate)
					if err != nil {
						return err
					}

					cnID, err := strconv.ParseUint(meta.ID, 10, 32)
					if err != nil {
						return err
					}

					err = addMeta(mp4.BoxType{'c', 'n', 'I', 'D'}, uint32(cnID))
					if err != nil {
						return err
					}

					err = addExtendedMeta("ISRC", meta.Attributes.ISRC)
					if err != nil {
						return err
					}

					if len(meta.Attributes.GenreNames) > 0 {
						err = addMeta(mp4.BoxType{'\251', 'g', 'e', 'n'}, meta.Attributes.GenreNames[0])
						if err != nil {
							return err
						}
					}

					if len(albums) > 0 {

						err = addMeta(mp4.BoxType{'a', 'A', 'R', 'T'}, meta.Attributes.ArtistName)
						if err != nil {
							return err
						}

						err = addMeta(mp4.BoxType{'s', 'o', 'a', 'a'}, meta.Attributes.ArtistName)
						if err != nil {
							return err
						}

						err = addMeta(mp4.BoxType{'c', 'p', 'r', 't'}, albums[0].Attributes.Copyright)
						if err != nil {
							return err
						}

						var isCpil uint8
						if albums[0].Attributes.IsCompilation {
							isCpil = 1
						}
						err = addMeta(mp4.BoxType{'c', 'p', 'i', 'l'}, isCpil)
						if err != nil {
							return err
						}

						err = addMeta(mp4.BoxType{'\251', 'p', 'u', 'b'}, albums[0].Attributes.RecordLabel)
						if err != nil {
							return err
						}

						err = addExtendedMeta("LABEL", albums[0].Attributes.RecordLabel)
						if err != nil {
							return err
						}

						err = addExtendedMeta("UPC", albums[0].Attributes.UPC)
						if err != nil {
							return err
						}

						//if !strings.Contains(meta.Data[0].ID, "pl.") {
						//	plID, err := strconv.ParseUint(meta.Data[0].ID, 10, 32)
						//	if err != nil {
						//		return err
						//	}
						//
						//	err = addMeta(mp4.BoxType{'p', 'l', 'I', 'D'}, uint32(plID))
						//	if err != nil {
						//		return err
						//	}
						//}
					}

					if len(artists) > 0 {
						if len(artists[0].ID) > 0 {
							atID, err := strconv.ParseUint(artists[0].ID, 10, 32)
							if err != nil {
								return err
							}

							err = addMeta(mp4.BoxType{'a', 't', 'I', 'D'}, uint32(atID))
							if err != nil {
								return err
							}
						}
					}
					trkn := make([]byte, 8)
					disk := make([]byte, 8)
					binary.BigEndian.PutUint32(trkn, uint32(meta.Attributes.TrackNumber))
					binary.BigEndian.PutUint16(trkn[4:], uint16(albums[0].Attributes.TrackCount))
					binary.BigEndian.PutUint32(disk, uint32(meta.Attributes.DiscNumber))
					//binary.BigEndian.PutUint16(disk[4:], uint16(meta.Data[0].Relationships.Tracks.Data[trackTotal-1].Attributes.DiscNumber))
					//if strings.Contains(meta.Data[0].ID, "pl.") {
					//	if !config.UseSongInfoForPlaylist {
					//		binary.BigEndian.PutUint32(trkn, uint32(trackNum))
					//		binary.BigEndian.PutUint16(trkn[4:], uint16(trackTotal))
					//		binary.BigEndian.PutUint32(disk, uint32(1))
					//		binary.BigEndian.PutUint16(disk[4:], uint16(1))
					//	}
					//}
					err = addMeta(mp4.BoxType{'t', 'r', 'k', 'n'}, trkn)
					if err != nil {
						return err
					}
					err = addMeta(mp4.BoxType{'d', 'i', 's', 'k'}, disk)
					if err != nil {
						return err
					}

					ctx.UnderIlst = false

					_, err = w.EndBox()
					if err != nil {
						return err
					}
				}

				ctx.UnderIlstMeta = false
				_, err = w.EndBox()
				if err != nil {
					return err
				}
			}

			ctx.UnderUdta = false
			_, err = w.EndBox()
			if err != nil {
				return err
			}
		}

		_, err = w.EndBox()
		if err != nil {
			return err
		}
	}

	{
		box, err := w.StartBox(&mp4.BoxInfo{Type: mp4.BoxTypeMdat()})
		if err != nil {
			return err
		}

		_, err = mp4.Marshal(w, &mp4.Mdat{Data: data}, box.Context)
		if err != nil {
			return err
		}

		mdat, err := w.EndBox()

		var realStco mp4.Stco

		offset := mdat.Offset + mdat.HeaderSize
		for i := uint32(0); i < numSamples; i++ {
			if i%chunkSize == 0 {
				realStco.EntryCount++
				realStco.ChunkOffset = append(realStco.ChunkOffset, uint32(offset))
			}
			offset += uint64(len(info.samples[i].data))
		}

		_, err = stco.SeekToPayload(w)
		if err != nil {
			return err
		}
		_, err = mp4.Marshal(w, &realStco, box.Context)
		if err != nil {
			return err
		}
	}

	return nil

}
