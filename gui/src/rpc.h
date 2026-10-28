#pragma once
// Discord Rich Presence (Discord IPC, no external library).
// Presence text is "CerberusMCP GUI" + the current page / server.
// Enabled when config::c.discordRpc is set and config::c.discordId is non-empty.
namespace rpc {

void Init();                            // nothing blocking, called once from app::Init
void Tick(float dt);                    // connection state machine, called every frame
void SetActivity(const char* state);    // "CerberusMCP GUI" details + given state line
void Clear();                           // remove the activity
void Shutdown();                        // disconnect

}
