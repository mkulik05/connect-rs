import { Redis } from "@upstash/redis/cloudflare"

let get_free_wg_id = async (redis, room_id) => {
    let len = await redis.llen(room_id) + 1;
    return "10.8." + Math.floor(len / 255) + "." + len % 256
}

async function handleRequest(request, redis) {
    const upgradeHeader = request.headers.get('Upgrade');

    if (!upgradeHeader || upgradeHeader !== 'websocket') {
        console.log("stupid user")
        return new Response('Expected Upgrade: websocket', { status: 426 });
    }
    const wsPair = new WebSocketPair();
    const [client, server] = Object.values(wsPair);

    server.accept();

    server.addEventListener('message', async (event) => {

        console.log(`[message]: ${event.data}`);
        let data = JSON.parse(event.data);
        let room_name = data.room_name;
        let wg_ip = await get_free_wg_id(redis, room_name)
        server.send(JSON.stringify({"WgIpMsg" : wg_ip}))
        data.peer_info.wg_ip = wg_ip;
        await redis.lpush(room_name, JSON.stringify(data.peer_info));
        await redis.publish(room_name, JSON.stringify({"JoinReqMsg": data})) 
    });
    return new Response(null, {
        status: 101,
        webSocket: client,
    });
}

export default {
    async fetch(request, env) {
        let redis = Redis.fromEnv(env)
        return await handleRequest(request, redis)
    },
};