import { Redis } from "@upstash/redis/cloudflare"

let get_free_wg_id = async (redis, room_id) => {
    let len = await redis.llen(room_id) + 1;
    return "10.8." + Math.floor(len / 255) + "." + len % 256
}



export default {
    async fetch(request, env) {
        let redis = Redis.fromEnv(env)
        if (request.method === "POST") {
            let data = JSON.parse(await request.text());
            let room_name = data.room_name;
            let wg_ip = await get_free_wg_id(redis, room_name)
            data.peer_info.wg_ip = wg_ip;
            await redis.lpush(room_name, JSON.stringify(data.peer_info));
            await redis.publish(room_name, JSON.stringify({"JoinReqMsg": data})) 
            return new Response(JSON.stringify({"WgIpMsg" : wg_ip}));
          } else {
            return new Response('Expected post request', {
                status: 403,
            });
          }
    },
};