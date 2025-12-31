## 链表使用过程详细描述

### 初始状态（初始化后）
```
nodes[0] -> next=Some(1)  <- free_head 指向这里
nodes[1] -> next=Some(2)  <- 空闲节点链表
nodes[2] -> next=Some(3) 
nodes[3] -> next=None
head=None, tail=None  <- 用户数据链表为空
```

### 1. 第一次 push_back 操作
```
// push_back(数据A)
- free_head=0, 获取节点0
- next_free = nodes[0].next = Some(1)
- 更新节点0: nodes[0] = ListNode { data=数据A, next=None }
- 更新 free_head = Some(1)
- 因为链表为空: head=Some(0), tail=Some(0)

用户数据链表: [节点0(数据A)]
nodes[0] -> data=数据A, next=None  <- 用户数据链表头尾
nodes[1] -> next=Some(2)  <- free_head 指向这里
nodes[2] -> next=Some(3)
nodes[3] -> next=None
```

### 2. 第二次 push_back 操作
```
// push_back(数据B)
- free_head=1, 获取节点1
- next_free = nodes[1].next = Some(2)
- 更新节点1: nodes[1] = ListNode { data=数据B, next=None }
- 更新 free_head = Some(2)
- 链表非空: 更新 nodes[0].next = Some(1), tail=Some(1)

用户数据链表: [节点0(数据A)] -> [节点1(数据B)]
nodes[0] -> data=数据A, next=Some(1)  <- 用户数据链表头
nodes[1] -> data=数据B, next=None     <- 用户数据链表尾
nodes[2] -> next=Some(3)  <- free_head 指向这里
nodes[3] -> next=None
```

### 3. 第三次 push_back 操作
```
// push_back(数据C)
- free_head=2, 获取节点2
- next_free = nodes[2].next = Some(3)
- 更新节点2: nodes[2] = ListNode { data=数据C, next=None }
- 更新 free_head = Some(3)
- 链表非空: 更新 nodes[1].next = Some(2), tail=Some(2)

用户数据链表: [节点0(数据A)] -> [节点1(数据B)] -> [节点2(数据C)]
nodes[0] -> data=数据A, next=Some(1)  <- 用户数据链表头
nodes[1] -> data=数据B, next=Some(2)
nodes[2] -> data=数据C, next=None     <- 用户数据链表尾
nodes[3] -> next=None  <- free_head 指向这里
```

### 4. 第一次 pop_front 操作
```
// pop_front() -> 返回数据A
- head=0, 获取节点0的数据A
- 更新 head = nodes[0].next = Some(1)
- 将节点0返回空闲列表:
  - nodes[0] = ListNode { data=零值, next=Some(3) }  <- next指向原free_head
  - free_head = Some(0)  <- free_head指向节点0

用户数据链表: [节点1(数据B)] -> [节点2(数据C)]
nodes[0] -> data=零值, next=Some(3)  <- free_head 指向这里
nodes[1] -> data=数据B, next=Some(2)  <- 用户数据链表头
nodes[2] -> data=数据C, next=None     <- 用户数据链表尾
nodes[3] -> next=None
```

### 5. 第四次 push_back 操作
```
// push_back(数据D)
- free_head=0, 获取节点0 (复用之前释放的节点)
- next_free = nodes[0].next = Some(3)
- 更新节点0: nodes[0] = ListNode { data=数据D, next=None }
- 更新 free_head = Some(3)
- 链表非空: 更新 nodes[2].next = Some(0), tail=Some(0)

用户数据链表: [节点1(数据B)] -> [节点2(数据C)] -> [节点0(数据D)]
nodes[0] -> data=数据D, next=None     <- 用户数据链表尾
nodes[1] -> data=数据B, next=Some(2)  <- 用户数据链表头
nodes[2] -> data=数据C, next=Some(0)
nodes[3] -> next=None  <- free_head 指向这里
```

## 空闲节点链表的维护

空闲节点链表始终保持连接所有未被用户使用的节点，形成一个链表：
- `free_head` 指向下一个可用节点
- 每个空闲节点的 `next` 字段指向下一个空闲节点
- 当节点被 `push` 操作使用时，从空闲链表移除
- 当节点被 `pop` 操作释放时，重新加入空闲链表的头部

这种设计确保了：
1. **O(1) 时间复杂度**：分配和释放节点都是常数时间
2. **内存复用**：节点被高效地重复使用，避免内存碎片
3. **无动态分配**：所有节点都在初始化时分配，运行时无需动态内存管理
4. **内存安全**：通过索引而非指针访问，避免悬空指针问题