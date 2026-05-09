<template>
  <article class="panel panel-chat">
    <div class="panel-title">对话入口</div>
    <div class="chat-stack">
      <div class="chat-bubble chat-bubble-user">
        帮我搭一个适合调试 Rust agent runtime 的工作台。
      </div>
      <div class="chat-bubble chat-bubble-agent">
        已采用 Vue + Pinia 重建前端骨架，下一步可以直接承接 run_turn() 和工具调用轨迹。
      </div>
    </div>
    <div class="composer">
      <input class="composer-input" type="text" placeholder="下一阶段这里会接真实对话输入..." />
      <button class="composer-button" type="button">发送</button>
    </div>
  </article>
</template>
