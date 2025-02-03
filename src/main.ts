import { InputMedia, md } from '@mtcute/bun';
import { filters } from '@mtcute/dispatcher';
import * as mongoose from 'mongoose';

import auth from './auth';
import * as env from './env';
import { Chat } from './models/chat';
import { File } from './models/file';
import { addToQueue, taskQueue } from './queue';
import { downloadSong } from './song';
import { dp, tg } from './tg';
import { extractUrlMeta } from './utils';

console.log(env);

dp.onNewMessage(filters.command('start'), async msg => {
    await msg.replyText(
        'Welcome to apple bot. use /help to see how to use it.\n\nFor now this bot only works in permitted group.'
    );
});

dp.onNewMessage(filters.command('help'), async msg => {
    await msg.replyText(
        md`
            Available commands - \n/help - Get this message \n/song - Download a single song \n/album - Get album URLs\`(WIP)\`\n/playlist - Get playlist URLs\`(WIP)\`\n\nExample - \n\`/song https://music.apple.com/in/album/never-gonna-give-you-up/1559523357?i=1559523359\`\n\`/song https://music.apple.com/in/song/never-gonna-give-you-up/1559523359\`\n\n \`/album https://music.apple.com/in/album/3-originals/1559523357\`\n\n\`/playlist https://music.apple.com/library/playlist/p.vMO5kRQiX1xGMr\`
        `
    );
});

dp.onNewMessage(filters.command('authorize'), async msg => {
    if (msg.sender.id.toString() !== env.ADMIN_ID) return;
    const repliedMsg = await msg.getReplyTo();
    if (repliedMsg) {
        await Chat.authorize(repliedMsg.sender.id);
        await msg.replyText('User authorized.', { replyTo: msg });
        return;
    }
    const args = msg.command;
    if (args.length < 2) {
        await Chat.authorize(msg.chat.id);
        await msg.replyText('Chat authorized.', { replyTo: msg });
        return;
    }
    const chatIds = args.slice(1).map(Number).filter(Boolean);
    for (const chatId of chatIds) {
        await Chat.authorize(chatId);
    }

    const reply = `${chatIds.length} Chat${
        chatIds.length > 1 ? 's' : ''
    } authorized.`;
    await msg.replyText(reply);
});

dp.onNewMessage(filters.command('id'), async msg => {
    const repliedMsg = await msg.getReplyTo();
    if (repliedMsg) {
        await msg.replyText(
            md`User id: \`${repliedMsg.sender.id}\`\n(Click/Tap to copy)`
        );
        return;
    }
    await msg.replyText(
        md`Chat id: \`${msg.chat.id.toString()}\`\n(Click/Tap to copy)`
    );
});

dp.onNewMessage(filters.and(filters.command('song'), auth), async msg => {
    const args = msg.command;
    if (args.length < 2) {
        await msg.replyText('Please provide a song URL.');
        return;
    }

    // check if exists in db
    const urlMeta = extractUrlMeta(args[1]);
    if (!urlMeta) {
        await msg.replyText('Invalid URL');
        return;
    }
    const file = await File.findOne({ appleId: urlMeta.id, type: 'song' });
    if (file) {
        for (const fileId of file.tgFileId) {
            await msg.replyMedia(InputMedia.audio(fileId));
        }
        return;
    }

    const senderId = msg.sender.id;
    const chatId = msg.chat.id;
    const messageId = msg.id;
    const uniqueId = `${senderId}:${chatId}:${messageId}`;

    const position = taskQueue.length + 1;

    if (position > 1) {
        await msg.replyText(
            `Your request has been added to the queue. Your position is ${position}.`
        );
    } else {
        await msg.replyText('Your request is being processed.');
    }

    // Add the song task to the queue
    addToQueue(uniqueId, async () => downloadSong(msg));
});
dp.onNewMessage(filters.command('ping'), async msg => {
    console.log(msg);
    await msg.replyText('Pong!');
    await msg.answerMedia(
        InputMedia.auto(
            'CQACAgUAAxkDAAIEHGdvuZKcz_W3Z23L8eCPj9coINJiAAJ-GAACPYF5V0J-80wAAc1QcjYE'
        )
    );
});

dp.onError(async (error, update, state) => {
    // if (update.name === 'new_message') {
    //     await update.data.replyText(`Error: ${error.message}`);

    //     return true;
    // }

    console.error('Error:', error.message);

    return false;
});

// Start the bot
mongoose
    .connect(env.MONGO_URI)
    .then(async () => {
        console.log('Connected to MongoDB');
        const user = await tg.start({ botToken: env.BOT_TOKEN });
        console.log('Logged in as', user.username);
    })
    .catch(err => {
        console.error('Error connecting to MongoDB:', err);
        process.exit(1);
    });
