import { filters, MessageContext } from '@mtcute/dispatcher';
import * as env from './env';
import { Chat } from './models/chat';

export default async function auth(message: unknown) {
    const msg = message as filters.Modify<
        MessageContext,
        { command: string[] }
    >;
    if (msg.sender.id.toString() === env.ADMIN_ID) return true;

    const senderId = msg.sender.id;
    const chatId = msg.chat.id;
    const chat = await Chat.findOne({ chatId: { $in: [chatId, senderId] } });
    return !!chat;
}
