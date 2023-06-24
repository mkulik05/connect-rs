import { Redis } from "@upstash/redis/cloudflare"

const WG_IP_PATTERN = "10.8.";
const MAX_INACTIVE_DELAY = 24 * 1000 * 3600;
const MAX_PEERS_N = 200;

let get_free_wg_id = (num) => {
    num += 1;
    return WG_IP_PATTERN + Math.floor(num / 255) + "." + num % 256
}

export default {
    async fetch(request, env) {
        let redis = Redis.fromEnv(env)
        if (request.method === "POST") {
            let data = JSON.parse(await request.text());
            console.log(data)
            if (data.type === "JoinMsg") {
                let room_name = data.room_name;
                let pub_key = data.peer_info.pub_key;
                let len = await redis.llen(room_name);
                if (len >= MAX_PEERS_N) {
                    return new Response('Room is full', {
                        status: 403,
                    });            
                }
                let peers = await redis.lrange(room_name, 0, len - 1);
                let res_index = -1;
                for (let i = peers.length - 1; i >= 0; i--) {
                    if (((new Date()) - (new Date(peers[i].last_connected))) > MAX_INACTIVE_DELAY) {
                        res_index = i;
                    }
                    if (peers[i].pub_key === pub_key) {
                        res_index = i;
                        break
                    }
                }
                let num = (res_index === -1) ? len : len - 1 - res_index;
                let wg_ip = get_free_wg_id(num);
                data.peer_info.wg_ip = wg_ip;
                data.peer_info.last_connected = new Date();
                if (res_index === -1) {
                    await redis.lpush(room_name, JSON.stringify(data.peer_info));
                } else {
                    await redis.lset(room_name, res_index, JSON.stringify(data.peer_info));
                }
                await redis.publish(room_name, JSON.stringify(data))
                return new Response(wg_ip);
            } else if (data.type === "DisconnectMsg") {
                await redis.publish(data.room_name, JSON.stringify(data));
                return new Response('OK');
            } else if (data.type === "UpdateLastConnected") {
                let len = await redis.llen(data.room_name);
                let peers = await redis.lrange(data.room_name, 0, len - 1);
                let res_i = -1;
                for (let i = 0; i < peers.length; i++) {
                    if (data.pub_key === peers[i].pub_key) {
                        res_i = i
                        break
                    }
                }
                if (res_i !== -1) {
                    let res_data = peers[res_i];
                    res_data.last_connected = new Date();
                    await redis.lset(data.room_name, res_i, JSON.stringify(res_data)); 
                }
                return new Response('OK');
            }
            return new Response('invalid operation', {
                status: 403,
            });
        } else {
            return new Response('Expected post request', {
                status: 403,
            });
        }
    },
};