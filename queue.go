package main

import (
	"fmt"
	"regexp"
	"sync"

	tg "gopkg.in/telebot.v4"
)

type CommandRequest struct {
	Command string
	Context tg.Context
	ID      string
}

var (
	commandQueue = make(chan CommandRequest, 100) // Channel for the command queue
	queueMutex   sync.Mutex
	queue        []CommandRequest
)

func init() {
	go processQueue() // Start the queue processing in a separate goroutine
}

func processQueue() {
	for {
		select {
		case req := <-commandQueue:
			go handleCommand(req)
		}
	}
}

func handleCommand(req CommandRequest) {
	queueMutex.Lock()
	defer queueMutex.Unlock()

	var err error
	switch req.Command {
	case "song":
		err = DownloadSong(req.Context)
		// case "playlist":
		// 	err = DownloadPlaylist(req.Context)
	default:
		err = fmt.Errorf("unknown command: %s", req.Command)
	}
	removeFromQueue(req.ID)
	//for i, req := range queue {
	//	if req.ID == ID {
	//		// Remove the request by index
	//		queue = append(queue[:i], queue[i+1:]...)
	//		break
	//	}
	//}

	if err != nil {
		er := regexp.MustCompile(`https?://\S+`).ReplaceAllString(err.Error(), "")
		_ = req.Context.Send(fmt.Sprintf("Error processing command: %s", er))
		return
	}
}

func removeFromQueue(ID string) {
	for i, req := range queue {
		if req.ID == ID {
			// Remove the request by index
			queue = append(queue[:i], queue[i+1:]...)
			break
		}
	}
}
