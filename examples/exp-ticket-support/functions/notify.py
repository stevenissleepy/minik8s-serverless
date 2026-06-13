import json


def new():
    return Function()


class Function:
    async def handle(self, scope, receive, send):
        message = await receive()
        body = message.get("body", b"")
        event = json.loads(body.decode("utf-8")) if body.strip() else {}
        ticket_id = event.get("ticket_id", "unknown")
        action = event.get("action", "unknown")

        result = {
            "ticket_id": ticket_id,
            "status": "notified",
            "action": action,
            "category": event.get("category"),
            "risk": event.get("risk"),
            "message": f"ticket {ticket_id} handled by {action}",
            "notified_by": scope["minik8s"]["name"],
        }
        await respond(send, result)


async def respond(send, result):
    payload = json.dumps(result).encode("utf-8")
    await send({
        "type": "http.response.start",
        "status": 200,
        "headers": [[b"content-type", b"application/json"]],
    })
    await send({"type": "http.response.body", "body": payload})
